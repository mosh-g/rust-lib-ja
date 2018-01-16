// Copyright 2012-2014 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! An analysis to determine which locals require allocas and
//! which do not.

use rustc_data_structures::bitvec::BitVector;
use rustc_data_structures::indexed_vec::{Idx, IndexVec};
use rustc::middle::const_val::ConstVal;
use rustc::mir::{self, Location, TerminatorKind, Literal};
use rustc::mir::visit::{Visitor, PlaceContext};
use rustc::mir::traversal;
use rustc::mir::interpret::{Value, PrimVal};
use rustc::ty;
use rustc::ty::layout::LayoutOf;
use type_of::LayoutLlvmExt;
use super::FunctionCx;

pub fn memory_locals<'a, 'tcx>(fx: &FunctionCx<'a, 'tcx>) -> BitVector {
    let mir = fx.mir;
    let mut analyzer = LocalAnalyzer::new(fx);

    analyzer.visit_mir(mir);

    for (index, ty) in mir.local_decls.iter().map(|l| l.ty).enumerate() {
        let ty = fx.monomorphize(&ty);
        debug!("local {} has type {:?}", index, ty);
        let layout = fx.cx.layout_of(ty);
        if layout.is_llvm_immediate() {
            // These sorts of types are immediates that we can store
            // in an ValueRef without an alloca.
        } else if layout.is_llvm_scalar_pair() {
            // We allow pairs and uses of any of their 2 fields.
        } else {
            // These sorts of types require an alloca. Note that
            // is_llvm_immediate() may *still* be true, particularly
            // for newtypes, but we currently force some types
            // (e.g. structs) into an alloca unconditionally, just so
            // that we don't have to deal with having two pathways
            // (gep vs extractvalue etc).
            analyzer.mark_as_memory(mir::Local::new(index));
        }
    }

    analyzer.memory_locals
}

struct LocalAnalyzer<'mir, 'a: 'mir, 'tcx: 'a> {
    fx: &'mir FunctionCx<'a, 'tcx>,
    memory_locals: BitVector,
    seen_assigned: BitVector
}

impl<'mir, 'a, 'tcx> LocalAnalyzer<'mir, 'a, 'tcx> {
    fn new(fx: &'mir FunctionCx<'a, 'tcx>) -> LocalAnalyzer<'mir, 'a, 'tcx> {
        let mut analyzer = LocalAnalyzer {
            fx,
            memory_locals: BitVector::new(fx.mir.local_decls.len()),
            seen_assigned: BitVector::new(fx.mir.local_decls.len())
        };

        // Arguments get assigned to by means of the function being called
        for idx in 0..fx.mir.arg_count {
            analyzer.seen_assigned.insert(idx + 1);
        }

        analyzer
    }

    fn mark_as_memory(&mut self, local: mir::Local) {
        debug!("marking {:?} as memory", local);
        self.memory_locals.insert(local.index());
    }

    fn mark_assigned(&mut self, local: mir::Local) {
        if !self.seen_assigned.insert(local.index()) {
            self.mark_as_memory(local);
        }
    }
}

impl<'mir, 'a, 'tcx> Visitor<'tcx> for LocalAnalyzer<'mir, 'a, 'tcx> {
    fn visit_assign(&mut self,
                    block: mir::BasicBlock,
                    place: &mir::Place<'tcx>,
                    rvalue: &mir::Rvalue<'tcx>,
                    location: Location) {
        debug!("visit_assign(block={:?}, place={:?}, rvalue={:?})", block, place, rvalue);

        if let mir::Place::Local(index) = *place {
            self.mark_assigned(index);
            if !self.fx.rvalue_creates_operand(rvalue) {
                self.mark_as_memory(index);
            }
        } else {
            self.visit_place(place, PlaceContext::Store, location);
        }

        self.visit_rvalue(rvalue, location);
    }

    fn visit_terminator_kind(&mut self,
                             block: mir::BasicBlock,
                             kind: &mir::TerminatorKind<'tcx>,
                             location: Location) {
        let check = match *kind {
            mir::TerminatorKind::Call {
                func: mir::Operand::Constant(box mir::Constant {
                    literal: Literal::Value {
                        value: &ty::Const { val, ty }, ..
                    }, ..
                }),
                ref args, ..
            } => match val {
                ConstVal::Value(Value::ByVal(PrimVal::Undef)) => match ty.sty {
                    ty::TyFnDef(did, _) => Some((did, args)),
                    _ => None,
                },
                _ => None,
            }
            _ => None,
        };
        if let Some((def_id, args)) = check {
            if Some(def_id) == self.fx.cx.tcx.lang_items().box_free_fn() {
                // box_free(x) shares with `drop x` the property that it
                // is not guaranteed to be statically dominated by the
                // definition of x, so x must always be in an alloca.
                if let mir::Operand::Move(ref place) = args[0] {
                    self.visit_place(place, PlaceContext::Drop, location);
                }
            }
        }

        self.super_terminator_kind(block, kind, location);
    }

    fn visit_place(&mut self,
                    place: &mir::Place<'tcx>,
                    context: PlaceContext<'tcx>,
                    location: Location) {
        debug!("visit_place(place={:?}, context={:?})", place, context);
        let cx = self.fx.cx;

        if let mir::Place::Projection(ref proj) = *place {
            // Allow uses of projections that are ZSTs or from scalar fields.
            let is_consume = match context {
                PlaceContext::Copy | PlaceContext::Move => true,
                _ => false
            };
            if is_consume {
                let base_ty = proj.base.ty(self.fx.mir, cx.tcx);
                let base_ty = self.fx.monomorphize(&base_ty);

                // ZSTs don't require any actual memory access.
                let elem_ty = base_ty.projection_ty(cx.tcx, &proj.elem).to_ty(cx.tcx);
                let elem_ty = self.fx.monomorphize(&elem_ty);
                if cx.layout_of(elem_ty).is_zst() {
                    return;
                }

                if let mir::ProjectionElem::Field(..) = proj.elem {
                    let layout = cx.layout_of(base_ty.to_ty(cx.tcx));
                    if layout.is_llvm_immediate() || layout.is_llvm_scalar_pair() {
                        // Recurse with the same context, instead of `Projection`,
                        // potentially stopping at non-operand projections,
                        // which would trigger `mark_as_memory` on locals.
                        self.visit_place(&proj.base, context, location);
                        return;
                    }
                }
            }

            // A deref projection only reads the pointer, never needs the place.
            if let mir::ProjectionElem::Deref = proj.elem {
                return self.visit_place(&proj.base, PlaceContext::Copy, location);
            }
        }

        self.super_place(place, context, location);
    }

    fn visit_local(&mut self,
                   &index: &mir::Local,
                   context: PlaceContext<'tcx>,
                   _: Location) {
        match context {
            PlaceContext::Call => {
                self.mark_assigned(index);
            }

            PlaceContext::StorageLive |
            PlaceContext::StorageDead |
            PlaceContext::Validate |
            PlaceContext::Copy |
            PlaceContext::Move => {}

            PlaceContext::Inspect |
            PlaceContext::Store |
            PlaceContext::AsmOutput |
            PlaceContext::Borrow { .. } |
            PlaceContext::Projection(..) => {
                self.mark_as_memory(index);
            }

            PlaceContext::Drop => {
                let ty = mir::Place::Local(index).ty(self.fx.mir, self.fx.cx.tcx);
                let ty = self.fx.monomorphize(&ty.to_ty(self.fx.cx.tcx));

                // Only need the place if we're actually dropping it.
                if self.fx.cx.type_needs_drop(ty) {
                    self.mark_as_memory(index);
                }
            }
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CleanupKind {
    NotCleanup,
    Funclet,
    Internal { funclet: mir::BasicBlock }
}

impl CleanupKind {
    pub fn funclet_bb(self, for_bb: mir::BasicBlock) -> Option<mir::BasicBlock> {
        match self {
            CleanupKind::NotCleanup => None,
            CleanupKind::Funclet => Some(for_bb),
            CleanupKind::Internal { funclet } => Some(funclet),
        }
    }
}

pub fn cleanup_kinds<'a, 'tcx>(mir: &mir::Mir<'tcx>) -> IndexVec<mir::BasicBlock, CleanupKind> {
    fn discover_masters<'tcx>(result: &mut IndexVec<mir::BasicBlock, CleanupKind>,
                              mir: &mir::Mir<'tcx>) {
        for (bb, data) in mir.basic_blocks().iter_enumerated() {
            match data.terminator().kind {
                TerminatorKind::Goto { .. } |
                TerminatorKind::Resume |
                TerminatorKind::Abort |
                TerminatorKind::Return |
                TerminatorKind::GeneratorDrop |
                TerminatorKind::Unreachable |
                TerminatorKind::SwitchInt { .. } |
                TerminatorKind::Yield { .. } |
                TerminatorKind::FalseEdges { .. } |
                TerminatorKind::FalseUnwind { .. } => {
                    /* nothing to do */
                }
                TerminatorKind::Call { cleanup: unwind, .. } |
                TerminatorKind::Assert { cleanup: unwind, .. } |
                TerminatorKind::DropAndReplace { unwind, .. } |
                TerminatorKind::Drop { unwind, .. } => {
                    if let Some(unwind) = unwind {
                        debug!("cleanup_kinds: {:?}/{:?} registering {:?} as funclet",
                               bb, data, unwind);
                        result[unwind] = CleanupKind::Funclet;
                    }
                }
            }
        }
    }

    fn propagate<'tcx>(result: &mut IndexVec<mir::BasicBlock, CleanupKind>,
                       mir: &mir::Mir<'tcx>) {
        let mut funclet_succs = IndexVec::from_elem(None, mir.basic_blocks());

        let mut set_successor = |funclet: mir::BasicBlock, succ| {
            match funclet_succs[funclet] {
                ref mut s @ None => {
                    debug!("set_successor: updating successor of {:?} to {:?}",
                           funclet, succ);
                    *s = Some(succ);
                },
                Some(s) => if s != succ {
                    span_bug!(mir.span, "funclet {:?} has 2 parents - {:?} and {:?}",
                              funclet, s, succ);
                }
            }
        };

        for (bb, data) in traversal::reverse_postorder(mir) {
            let funclet = match result[bb] {
                CleanupKind::NotCleanup => continue,
                CleanupKind::Funclet => bb,
                CleanupKind::Internal { funclet } => funclet,
            };

            debug!("cleanup_kinds: {:?}/{:?}/{:?} propagating funclet {:?}",
                   bb, data, result[bb], funclet);

            for &succ in data.terminator().successors().iter() {
                let kind = result[succ];
                debug!("cleanup_kinds: propagating {:?} to {:?}/{:?}",
                       funclet, succ, kind);
                match kind {
                    CleanupKind::NotCleanup => {
                        result[succ] = CleanupKind::Internal { funclet: funclet };
                    }
                    CleanupKind::Funclet => {
                        if funclet != succ {
                            set_successor(funclet, succ);
                        }
                    }
                    CleanupKind::Internal { funclet: succ_funclet } => {
                        if funclet != succ_funclet {
                            // `succ` has 2 different funclet going into it, so it must
                            // be a funclet by itself.

                            debug!("promoting {:?} to a funclet and updating {:?}", succ,
                                   succ_funclet);
                            result[succ] = CleanupKind::Funclet;
                            set_successor(succ_funclet, succ);
                            set_successor(funclet, succ);
                        }
                    }
                }
            }
        }
    }

    let mut result = IndexVec::from_elem(CleanupKind::NotCleanup, mir.basic_blocks());

    discover_masters(&mut result, mir);
    propagate(&mut result, mir);
    debug!("cleanup_kinds: result={:?}", result);
    result
}
