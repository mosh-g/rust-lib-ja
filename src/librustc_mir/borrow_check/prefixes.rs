//! From the NLL RFC: "The deep [aka 'supporting'] prefixes for an
//! place are formed by stripping away fields and derefs, except that
//! we stop when we reach the deref of a shared reference. [...] "
//!
//! "Shallow prefixes are found by stripping away fields, but stop at
//! any dereference. So: writing a path like `a` is illegal if `a.b`
//! is borrowed. But: writing `a` is legal if `*a` is borrowed,
//! whether or not `a` is a shared or mutable reference. [...] "

use super::MirBorrowckCtxt;

use rustc::hir;
use rustc::ty::{self, TyCtxt};
use rustc::mir::{Body, Place, PlaceBase, Projection, ProjectionElem};

pub trait IsPrefixOf<'tcx> {
    fn is_prefix_of(&self, other: &Place<'tcx>) -> bool;
}

impl<'tcx> IsPrefixOf<'tcx> for Place<'tcx> {
    fn is_prefix_of(&self, other: &Place<'tcx>) -> bool {
        let mut cursor = &other.projection;
        loop {
            if self.projection == *cursor {
                return self.base == other.base;
            }

            match cursor {
                None => return false,
                Some(proj) => cursor = &proj.base,
            }
        }
    }
}

pub(super) struct Prefixes<'cx, 'tcx> {
    body: &'cx Body<'tcx>,
    tcx: TyCtxt<'tcx>,
    kind: PrefixSet,
    next: Option<(&'cx PlaceBase<'tcx>, &'cx Option<Box<Projection<'tcx>>>)>,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[allow(dead_code)]
pub(super) enum PrefixSet {
    /// Doesn't stop until it returns the base case (a Local or
    /// Static prefix).
    All,
    /// Stops at any dereference.
    Shallow,
    /// Stops at the deref of a shared reference.
    Supporting,
}

impl<'cx, 'tcx> MirBorrowckCtxt<'cx, 'tcx> {
    /// Returns an iterator over the prefixes of `place`
    /// (inclusive) from longest to smallest, potentially
    /// terminating the iteration early based on `kind`.
    pub(super) fn prefixes(
        &self,
        place_base: &'cx PlaceBase<'tcx>,
        place_projection: &'cx Option<Box<Projection<'tcx>>>,
        kind: PrefixSet,
    ) -> Prefixes<'cx, 'tcx> {
        Prefixes {
            next: Some((place_base, place_projection)),
            kind,
            body: self.body,
            tcx: self.infcx.tcx,
        }
    }
}

impl<'cx, 'tcx> Iterator for Prefixes<'cx, 'tcx> {
    type Item = (&'cx PlaceBase<'tcx>, &'cx Option<Box<Projection<'tcx>>>);
    fn next(&mut self) -> Option<Self::Item> {
        let mut cursor = self.next?;

        // Post-processing `place`: Enqueue any remaining
        // work. Also, `place` may not be a prefix itself, but
        // may hold one further down (e.g., we never return
        // downcasts here, but may return a base of a downcast).

        'cursor: loop {
            let proj = match cursor {
                (&PlaceBase::Local(_), &None)
                | // search yielded this leaf
                (&PlaceBase::Static(_), &None) => {
                    self.next = None;
                    return Some(cursor);
                }
                (_, &Some(ref proj))  => proj,
            };

            match proj.elem {
                ProjectionElem::Field(_ /*field*/, _ /*ty*/) => {
                    // FIXME: add union handling
                    self.next = Some((cursor.0, &proj.base));
                    return Some(cursor);
                }
                ProjectionElem::Downcast(..) |
                ProjectionElem::Subslice { .. } |
                ProjectionElem::ConstantIndex { .. } |
                ProjectionElem::Index(_) => {
                    cursor = (cursor.0, &proj.base);
                    continue 'cursor;
                }
                ProjectionElem::Deref => {
                    // (handled below)
                }
            }

            assert_eq!(proj.elem, ProjectionElem::Deref);

            match self.kind {
                PrefixSet::Shallow => {
                    // shallow prefixes are found by stripping away
                    // fields, but stop at *any* dereference.
                    // So we can just stop the traversal now.
                    self.next = None;
                    return Some(cursor);
                }
                PrefixSet::All => {
                    // all prefixes: just blindly enqueue the base
                    // of the projection
                    self.next = Some((cursor.0, &proj.base));
                    return Some(cursor);
                }
                PrefixSet::Supporting => {
                    // fall through!
                }
            }

            assert_eq!(self.kind, PrefixSet::Supporting);
            // supporting prefixes: strip away fields and
            // derefs, except we stop at the deref of a shared
            // reference.

            let ty = Place::ty_from(cursor.0, &proj.base, self.body, self.tcx).ty;
            match ty.sty {
                ty::RawPtr(_) |
                ty::Ref(
                    _, /*rgn*/
                    _, /*ty*/
                    hir::MutImmutable
                    ) => {
                    // don't continue traversing over derefs of raw pointers or shared borrows.
                    self.next = None;
                    return Some(cursor);
                }

                ty::Ref(
                    _, /*rgn*/
                    _, /*ty*/
                    hir::MutMutable,
                    ) => {
                    self.next = Some((cursor.0, &proj.base));
                    return Some(cursor);
                }

                ty::Adt(..) if ty.is_box() => {
                    self.next = Some((cursor.0, &proj.base));
                    return Some(cursor);
                }

                _ => panic!("unknown type fed to Projection Deref."),
            }
        }
    }
}
