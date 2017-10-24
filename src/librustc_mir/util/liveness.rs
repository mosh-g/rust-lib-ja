// Copyright 2017 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Liveness analysis which computes liveness of MIR local variables at the boundary of basic blocks
//!
//! This analysis considers references as being used only at the point of the
//! borrow. This means that this does not track uses because of references that
//! already exist:
//!
//! ```Rust
//!     fn foo() {
//!         x = 0;
//!         // `x` is live here
//!         GLOBAL = &x: *const u32;
//!         // but not here, even while it can be accessed through `GLOBAL`.
//!         foo();
//!         x = 1;
//!         // `x` is live again here, because it is assigned to `OTHER_GLOBAL`
//!         OTHER_GLOBAL = &x: *const u32;
//!         // ...
//!     }
//! ```
//!
//! This means that users of this analysis still have to check whether
//! pre-existing references can be used to access the value (e.g. at movable
//! generator yield points, all pre-existing references are invalidated, so this
//! doesn't matter).

use rustc::mir::*;
use rustc::mir::visit::{LvalueContext, Visitor};
use rustc_data_structures::indexed_vec::{IndexVec, Idx};
use rustc_data_structures::indexed_set::IdxSetBuf;
use util::pretty::{write_basic_block, dump_enabled, write_mir_intro};
use rustc::mir::transform::MirSource;
use rustc::ty::item_path;
use std::path::{PathBuf, Path};
use std::fs;
use rustc::ty::TyCtxt;
use std::io::{self, Write};

pub type LocalSet = IdxSetBuf<Local>;

#[derive(Eq, PartialEq, Clone)]
struct DefsUses {
    defs: LocalSet,
    uses: LocalSet,
}

impl DefsUses {
    fn apply(&self, bits: &mut LocalSet) -> bool {
        bits.subtract(&self.defs) | bits.union(&self.uses)
    }

    fn add_def(&mut self, index: Local) {
        // If it was used already in the block, remove that use
        // now that we found a definition.
        //
        // Example:
        //
        //     // Defs = {X}, Uses = {}
        //     X = 5
        //     // Defs = {}, Uses = {X}
        //     use(X)
        self.uses.remove(&index);
        self.defs.add(&index);
    }

    fn add_use(&mut self, index: Local) {
        // Inverse of above.
        //
        // Example:
        //
        //     // Defs = {}, Uses = {X}
        //     use(X)
        //     // Defs = {X}, Uses = {}
        //     X = 5
        //     // Defs = {}, Uses = {X}
        //     use(X)
        self.defs.remove(&index);
        self.uses.add(&index);
    }
}

impl<'tcx> Visitor<'tcx> for DefsUses {
    fn visit_local(&mut self,
                   &local: &Local,
                   context: LvalueContext<'tcx>,
                   _: Location) {
        match context {
            ///////////////////////////////////////////////////////////////////////////
            // DEFS

            LvalueContext::Store |

            // We let Call defined the result in both the success and
            // unwind cases. This is not really correct, however it
            // does not seem to be observable due to the way that we
            // generate MIR. See the test case
            // `mir-opt/nll/liveness-call-subtlety.rs`. To do things
            // properly, we would apply the def in call only to the
            // input from the success path and not the unwind
            // path. -nmatsakis
            LvalueContext::Call |

            // Storage live and storage dead aren't proper defines, but we can ignore
            // values that come before them.
            LvalueContext::StorageLive |
            LvalueContext::StorageDead => {
                self.add_def(local);
            }

            ///////////////////////////////////////////////////////////////////////////
            // USES

            LvalueContext::Projection(..) |

            // Borrows only consider their local used at the point of the borrow.
            // This won't affect the results since we use this analysis for generators
            // and we only care about the result at suspension points. Borrows cannot
            // cross suspension points so this behavior is unproblematic.
            LvalueContext::Borrow { .. } |

            LvalueContext::Inspect |
            LvalueContext::Consume |
            LvalueContext::Validate |

            // We consider drops to always be uses of locals.
            // Drop eloboration should be run before this analysis otherwise
            // the results might be too pessimistic.
            LvalueContext::Drop => {
                self.add_use(local);
            }
        }
    }
}

fn block<'tcx>(b: &BasicBlockData<'tcx>, locals: usize) -> DefsUses {
    let mut visitor = DefsUses {
        defs: LocalSet::new_empty(locals),
        uses: LocalSet::new_empty(locals),
    };

    let dummy_location = Location { block: BasicBlock::new(0), statement_index: 0 };

    // Visit the various parts of the basic block in reverse. If we go
    // forward, the logic in `add_def` and `add_use` would be wrong.
    visitor.visit_terminator(BasicBlock::new(0), b.terminator(), dummy_location);
    for statement in b.statements.iter().rev() {
        visitor.visit_statement(BasicBlock::new(0), statement, dummy_location);
    }

    visitor
}

// This gives the result of the liveness analysis at the boundary of basic blocks
pub struct LivenessResult {
    pub ins: IndexVec<BasicBlock, LocalSet>,
    pub outs: IndexVec<BasicBlock, LocalSet>,
}

pub fn liveness_of_locals<'tcx>(mir: &Mir<'tcx>) -> LivenessResult {
    let locals = mir.local_decls.len();
    let def_use: IndexVec<_, _> = mir.basic_blocks().iter().map(|b| {
        block(b, locals)
    }).collect();

    let mut ins: IndexVec<_, _> = mir.basic_blocks()
        .indices()
        .map(|_| LocalSet::new_empty(locals))
        .collect();
    let mut outs = ins.clone();

    let mut changed = true;
    let mut bits = LocalSet::new_empty(locals);
    while changed {
        changed = false;

        for b in mir.basic_blocks().indices().rev() {
            // outs[b] = ∪ {ins of successors}
            bits.clear();
            for &successor in mir.basic_blocks()[b].terminator().successors().into_iter() {
                bits.union(&ins[successor]);
            }
            outs[b].clone_from(&bits);

            // bits = use ∪ (bits - def)
            def_use[b].apply(&mut bits);

            // update bits on entry and flag if they have changed
            if ins[b] != bits {
                ins[b].clone_from(&bits);
                changed = true;
            }
        }
    }

    LivenessResult {
        ins,
        outs,
    }
}

impl LivenessResult {
    /// Walks backwards through the statements/terminator in the given
    /// basic block `block`.  At each point within `block`, invokes
    /// the callback `op` with the current location and the set of
    /// variables that are live on entry to that location.
    pub fn simulate_block<'tcx, OP>(&self,
                                    mir: &Mir<'tcx>,
                                    block: BasicBlock,
                                    mut callback: OP)
        where OP: FnMut(Location, &LocalSet)
    {
        let data = &mir[block];

        // Get a copy of the bits on exit from the block.
        let mut bits = self.outs[block].clone();

        // Start with the maximal statement index -- i.e., right before
        // the terminator executes.
        let mut statement_index = data.statements.len();

        // Compute liveness right before terminator and invoke callback.
        let terminator_location = Location { block, statement_index };
        let terminator_defs_uses = self.defs_uses(mir, terminator_location, &data.terminator);
        terminator_defs_uses.apply(&mut bits);
        callback(terminator_location, &bits);

        // Compute liveness before each statement (in rev order) and invoke callback.
        for statement in data.statements.iter().rev() {
            statement_index -= 1;
            let statement_location = Location { block, statement_index };
            let statement_defs_uses = self.defs_uses(mir, statement_location, statement);
            statement_defs_uses.apply(&mut bits);
            callback(statement_location, &bits);
        }

        assert_eq!(bits, self.ins[block]);
    }

    fn defs_uses<'tcx, V>(&self,
                          mir: &Mir<'tcx>,
                          location: Location,
                          thing: &V)
                          -> DefsUses
        where V: MirVisitable<'tcx>,
    {
        let locals = mir.local_decls.len();
        let mut visitor = DefsUses {
            defs: LocalSet::new_empty(locals),
            uses: LocalSet::new_empty(locals),
        };

        // Visit the various parts of the basic block in reverse. If we go
        // forward, the logic in `add_def` and `add_use` would be wrong.
        thing.apply(location, &mut visitor);

        visitor
    }
}

trait MirVisitable<'tcx> {
    fn apply<V>(&self, location: Location, visitor: &mut V)
        where V: Visitor<'tcx>;
}

impl<'tcx> MirVisitable<'tcx> for Statement<'tcx> {
    fn apply<V>(&self, location: Location, visitor: &mut V)
        where V: Visitor<'tcx>
    {
        visitor.visit_statement(location.block,
                                self,
                                location)
    }
}

impl<'tcx> MirVisitable<'tcx> for Option<Terminator<'tcx>> {
    fn apply<V>(&self, location: Location, visitor: &mut V)
        where V: Visitor<'tcx>
    {
        visitor.visit_terminator(location.block,
                                 self.as_ref().unwrap(),
                                 location)
    }
}

pub fn dump_mir<'a, 'tcx>(tcx: TyCtxt<'a, 'tcx, 'tcx>,
                          pass_name: &str,
                          source: MirSource,
                          mir: &Mir<'tcx>,
                          result: &LivenessResult) {
    if !dump_enabled(tcx, pass_name, source) {
        return;
    }
    let node_path = item_path::with_forced_impl_filename_line(|| { // see notes on #41697 below
        tcx.item_path_str(tcx.hir.local_def_id(source.item_id()))
    });
    dump_matched_mir_node(tcx, pass_name, &node_path,
                          source, mir, result);
}

fn dump_matched_mir_node<'a, 'tcx>(tcx: TyCtxt<'a, 'tcx, 'tcx>,
                                   pass_name: &str,
                                   node_path: &str,
                                   source: MirSource,
                                   mir: &Mir<'tcx>,
                                   result: &LivenessResult) {
    let mut file_path = PathBuf::new();
    if let Some(ref file_dir) = tcx.sess.opts.debugging_opts.dump_mir_dir {
        let p = Path::new(file_dir);
        file_path.push(p);
    };
    let file_name = format!("rustc.node{}{}-liveness.mir",
                            source.item_id(), pass_name);
    file_path.push(&file_name);
    let _ = fs::File::create(&file_path).and_then(|mut file| {
        writeln!(file, "// MIR local liveness analysis for `{}`", node_path)?;
        writeln!(file, "// source = {:?}", source)?;
        writeln!(file, "// pass_name = {}", pass_name)?;
        writeln!(file, "")?;
        write_mir_fn(tcx, source, mir, &mut file, result)?;
        Ok(())
    });
}

pub fn write_mir_fn<'a, 'tcx>(tcx: TyCtxt<'a, 'tcx, 'tcx>,
                              src: MirSource,
                              mir: &Mir<'tcx>,
                              w: &mut Write,
                              result: &LivenessResult)
                              -> io::Result<()> {
    write_mir_intro(tcx, src, mir, w)?;
    for block in mir.basic_blocks().indices() {
        let print = |w: &mut Write, prefix, result: &IndexVec<BasicBlock, LocalSet>| {
            let live: Vec<String> = mir.local_decls.indices()
                .filter(|i| result[block].contains(i))
                .map(|i| format!("{:?}", i))
                .collect();
            writeln!(w, "{} {{{}}}", prefix, live.join(", "))
        };
        print(w, "   ", &result.ins)?;
        write_basic_block(tcx, block, mir, &mut |_, _| Ok(()), w)?;
        print(w, "   ", &result.outs)?;
        if block.index() + 1 != mir.basic_blocks().len() {
            writeln!(w, "")?;
        }
    }

    writeln!(w, "}}")?;
    Ok(())
}

