// Copyright 2012-2014 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use ty::{self, Lift, TyCtxt, Region};
use rustc_data_structures::transitive_relation::TransitiveRelation;

#[derive(Clone, RustcEncodable, RustcDecodable, Debug)]
pub struct FreeRegionMap<'tcx> {
    // Stores the relation `a < b`, where `a` and `b` are regions.
    //
    // Invariant: only free regions like `'x` or `'static` are stored
    // in this relation, not scopes.
    relation: TransitiveRelation<Region<'tcx>>
}

impl<'tcx> FreeRegionMap<'tcx> {
    pub fn new() -> Self {
        FreeRegionMap { relation: TransitiveRelation::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.relation.is_empty()
    }

    // Record that `'sup:'sub`. Or, put another way, `'sub <= 'sup`.
    // (with the exception that `'static: 'x` is not notable)
    pub fn relate_regions(&mut self, sub: Region<'tcx>, sup: Region<'tcx>) {
        debug!("relate_regions(sub={:?}, sup={:?})", sub, sup);
        if is_free_or_static(sub) && is_free(sup) {
            self.relation.add(sub, sup)
        }
    }

    /// Tests whether `r_a <= sup`. Both must be free regions or
    /// `'static`.
    pub fn sub_free_regions<'a, 'gcx>(&self,
                                      r_a: Region<'tcx>,
                                      r_b: Region<'tcx>)
                                      -> bool {
        assert!(is_free_or_static(r_a) && is_free_or_static(r_b));
        if let ty::ReStatic = r_b {
            true // `'a <= 'static` is just always true, and not stored in the relation explicitly
        } else {
            r_a == r_b || self.relation.contains(&r_a, &r_b)
        }
    }

    /// Compute the least-upper-bound of two free regions. In some
    /// cases, this is more conservative than necessary, in order to
    /// avoid making arbitrary choices. See
    /// `TransitiveRelation::postdom_upper_bound` for more details.
    pub fn lub_free_regions<'a, 'gcx>(&self,
                                      tcx: TyCtxt<'a, 'gcx, 'tcx>,
                                      r_a: Region<'tcx>,
                                      r_b: Region<'tcx>)
                                      -> Region<'tcx> {
        debug!("lub_free_regions(r_a={:?}, r_b={:?})", r_a, r_b);
        assert!(is_free(r_a));
        assert!(is_free(r_b));
        let result = if r_a == r_b { r_a } else {
            match self.relation.postdom_upper_bound(&r_a, &r_b) {
                None => tcx.mk_region(ty::ReStatic),
                Some(r) => *r,
            }
        };
        debug!("lub_free_regions(r_a={:?}, r_b={:?}) = {:?}", r_a, r_b, result);
        result
    }

    /// Returns all regions that are known to outlive `r_a`. For
    /// example, in a function:
    ///
    /// ```
    /// fn foo<'a, 'b: 'a, 'c: 'b>() { .. }
    /// ```
    ///
    /// if `r_a` represents `'a`, this function would return `{'b, 'c}`.
    pub fn regions_that_outlive<'a, 'gcx>(&self, r_a: Region<'tcx>) -> Vec<&Region<'tcx>> {
        assert!(is_free(r_a) || *r_a == ty::ReStatic);
        self.relation.greater_than(&r_a)
    }
}

fn is_free(r: Region) -> bool {
    match *r {
        ty::ReEarlyBound(_) | ty::ReFree(_) => true,
        _ => false
    }
}

fn is_free_or_static(r: Region) -> bool {
    match *r {
        ty::ReStatic => true,
        _ => is_free(r),
    }
}

impl_stable_hash_for!(struct FreeRegionMap<'tcx> {
    relation
});

impl<'a, 'tcx> Lift<'tcx> for FreeRegionMap<'a> {
    type Lifted = FreeRegionMap<'tcx>;
    fn lift_to_tcx<'b, 'gcx>(&self, tcx: TyCtxt<'b, 'gcx, 'tcx>) -> Option<FreeRegionMap<'tcx>> {
        self.relation.maybe_map(|&fr| fr.lift_to_tcx(tcx))
                     .map(|relation| FreeRegionMap { relation })
    }
}
