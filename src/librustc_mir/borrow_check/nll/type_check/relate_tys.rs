// Copyright 2017 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use borrow_check::nll::constraints::OutlivesConstraint;
use borrow_check::nll::type_check::{BorrowCheckContext, Locations};
use borrow_check::nll::universal_regions::UniversalRegions;
use borrow_check::nll::ToRegionVid;
use rustc::infer::canonical::{Canonical, CanonicalVarInfos};
use rustc::infer::{InferCtxt, NLLRegionVariableOrigin};
use rustc::traits::query::Fallible;
use rustc::ty::fold::{TypeFoldable, TypeFolder, TypeVisitor};
use rustc::ty::relate::{self, Relate, RelateResult, TypeRelation};
use rustc::ty::subst::Kind;
use rustc::ty::{self, CanonicalTy, CanonicalVar, RegionVid, Ty, TyCtxt};
use rustc_data_structures::fx::FxHashMap;
use rustc_data_structures::indexed_vec::IndexVec;
use std::mem;

pub(super) fn sub_types<'tcx>(
    infcx: &InferCtxt<'_, '_, 'tcx>,
    a: Ty<'tcx>,
    b: Ty<'tcx>,
    locations: Locations,
    borrowck_context: Option<&mut BorrowCheckContext<'_, 'tcx>>,
) -> Fallible<()> {
    debug!("sub_types(a={:?}, b={:?}, locations={:?})", a, b, locations);
    TypeRelating::new(
        infcx,
        ty::Variance::Covariant,
        locations,
        borrowck_context,
        ty::List::empty(),
    ).relate(&a, &b)?;
    Ok(())
}

pub(super) fn eq_types<'tcx>(
    infcx: &InferCtxt<'_, '_, 'tcx>,
    a: Ty<'tcx>,
    b: Ty<'tcx>,
    locations: Locations,
    borrowck_context: Option<&mut BorrowCheckContext<'_, 'tcx>>,
) -> Fallible<()> {
    debug!("eq_types(a={:?}, b={:?}, locations={:?})", a, b, locations);
    TypeRelating::new(
        infcx,
        ty::Variance::Invariant,
        locations,
        borrowck_context,
        ty::List::empty(),
    ).relate(&a, &b)?;
    Ok(())
}

pub(super) fn eq_canonical_type_and_type<'tcx>(
    infcx: &InferCtxt<'_, '_, 'tcx>,
    a: CanonicalTy<'tcx>,
    b: Ty<'tcx>,
    locations: Locations,
    borrowck_context: Option<&mut BorrowCheckContext<'_, 'tcx>>,
) -> Fallible<()> {
    debug!(
        "eq_canonical_type_and_type(a={:?}, b={:?}, locations={:?})",
        a, b, locations
    );
    let Canonical {
        variables: a_variables,
        value: a_value,
    } = a;
    TypeRelating::new(
        infcx,
        ty::Variance::Invariant,
        locations,
        borrowck_context,
        a_variables,
    ).relate(&a_value, &b)?;
    Ok(())
}

struct TypeRelating<'cx, 'bccx: 'cx, 'gcx: 'tcx, 'tcx: 'bccx> {
    infcx: &'cx InferCtxt<'cx, 'gcx, 'tcx>,

    /// How are we relating `a` and `b`?
    ///
    /// - covariant means `a <: b`
    /// - contravariant means `b <: a`
    /// - invariant means `a == b
    /// - bivariant means that it doesn't matter
    ambient_variance: ty::Variance,

    /// When we pass through a set of binders (e.g., when looking into
    /// a `fn` type), we push a new bound region scope onto here.  This
    /// will contain the instantiated region for each region in those
    /// binders. When we then encounter a `ReLateBound(d, br)`, we can
    /// use the debruijn index `d` to find the right scope, and then
    /// bound region name `br` to find the specific instantiation from
    /// within that scope. See `replace_bound_region`.
    ///
    /// This field stores the instantiations for late-bound regions in
    /// the `a` type.
    a_scopes: Vec<BoundRegionScope>,

    /// Same as `a_scopes`, but for the `b` type.
    b_scopes: Vec<BoundRegionScope>,

    /// Where (and why) is this relation taking place?
    locations: Locations,

    /// This will be `Some` when we are running the type check as part
    /// of NLL, and `None` if we are running a "sanity check".
    borrowck_context: Option<&'cx mut BorrowCheckContext<'bccx, 'tcx>>,

    /// As we execute, the type on the LHS *may* come from a canonical
    /// source. In that case, we will sometimes find a constraint like
    /// `?0 = B`, where `B` is a type from the RHS. The first time we
    /// find that, we simply record `B` (and the list of scopes that
    /// tells us how to *interpret* `B`). The next time we encounter
    /// `?0`, then, we can read this value out and use it.
    ///
    /// One problem: these variables may be in some other universe,
    /// how can we enforce that? I guess I could add some kind of
    /// "minimum universe constraint" that we can feed to the NLL checker.
    /// --> also, we know this doesn't happen
    canonical_var_values: IndexVec<CanonicalVar, Option<Kind<'tcx>>>,
}

#[derive(Clone, Debug)]
struct ScopesAndKind<'tcx> {
    scopes: Vec<BoundRegionScope>,
    kind: Kind<'tcx>,
}

#[derive(Clone, Debug, Default)]
struct BoundRegionScope {
    map: FxHashMap<ty::BoundRegion, RegionVid>,
}

#[derive(Copy, Clone)]
struct UniversallyQuantified(bool);

impl<'cx, 'bccx, 'gcx, 'tcx> TypeRelating<'cx, 'bccx, 'gcx, 'tcx> {
    fn new(
        infcx: &'cx InferCtxt<'cx, 'gcx, 'tcx>,
        ambient_variance: ty::Variance,
        locations: Locations,
        borrowck_context: Option<&'cx mut BorrowCheckContext<'bccx, 'tcx>>,
        canonical_var_infos: CanonicalVarInfos<'tcx>,
    ) -> Self {
        let canonical_var_values = IndexVec::from_elem_n(None, canonical_var_infos.len());
        Self {
            infcx,
            ambient_variance,
            borrowck_context,
            locations,
            canonical_var_values,
            a_scopes: vec![],
            b_scopes: vec![],
        }
    }

    fn ambient_covariance(&self) -> bool {
        match self.ambient_variance {
            ty::Variance::Covariant | ty::Variance::Invariant => true,
            ty::Variance::Contravariant | ty::Variance::Bivariant => false,
        }
    }

    fn ambient_contravariance(&self) -> bool {
        match self.ambient_variance {
            ty::Variance::Contravariant | ty::Variance::Invariant => true,
            ty::Variance::Covariant | ty::Variance::Bivariant => false,
        }
    }

    fn create_scope(
        &mut self,
        value: &ty::Binder<impl TypeFoldable<'tcx>>,
        universally_quantified: UniversallyQuantified,
    ) -> BoundRegionScope {
        let mut scope = BoundRegionScope::default();
        value.skip_binder().visit_with(&mut ScopeInstantiator {
            infcx: self.infcx,
            target_index: ty::INNERMOST,
            universally_quantified,
            bound_region_scope: &mut scope,
        });
        scope
    }

    /// When we encounter binders during the type traversal, we record
    /// the value to substitute for each of the things contained in
    /// that binder. (This will be either a universal placeholder or
    /// an existential inference variable.) Given the debruijn index
    /// `debruijn` (and name `br`) of some binder we have now
    /// encountered, this routine finds the value that we instantiated
    /// the region with; to do so, it indexes backwards into the list
    /// of ambient scopes `scopes`.
    fn lookup_bound_region(
        debruijn: ty::DebruijnIndex,
        br: &ty::BoundRegion,
        first_free_index: ty::DebruijnIndex,
        scopes: &[BoundRegionScope],
    ) -> RegionVid {
        // The debruijn index is a "reverse index" into the
        // scopes listing. So when we have INNERMOST (0), we
        // want the *last* scope pushed, and so forth.
        let debruijn_index = debruijn.index() - first_free_index.index();
        let scope = &scopes[scopes.len() - debruijn_index - 1];

        // Find this bound region in that scope to map to a
        // particular region.
        scope.map[br]
    }

    /// If `r` is a bound region, find the scope in which it is bound
    /// (from `scopes`) and return the value that we instantiated it
    /// with. Otherwise just return `r`.
    fn replace_bound_region(
        &self,
        universal_regions: &UniversalRegions<'tcx>,
        r: ty::Region<'tcx>,
        first_free_index: ty::DebruijnIndex,
        scopes: &[BoundRegionScope],
    ) -> RegionVid {
        match r {
            ty::ReLateBound(debruijn, br) => {
                Self::lookup_bound_region(*debruijn, br, first_free_index, scopes)
            }

            ty::ReVar(v) => *v,

            _ => universal_regions.to_region_vid(r),
        }
    }

    /// Push a new outlives requirement into our output set of
    /// constraints.
    fn push_outlives(&mut self, sup: RegionVid, sub: RegionVid) {
        debug!("push_outlives({:?}: {:?})", sup, sub);

        if let Some(borrowck_context) = &mut self.borrowck_context {
            borrowck_context
                .constraints
                .outlives_constraints
                .push(OutlivesConstraint {
                    sup,
                    sub,
                    locations: self.locations,
                });

            // FIXME all facts!
        }
    }

    /// When we encounter a canonical variable `var` in the output,
    /// equate it with `kind`. If the variable has been previously
    /// equated, then equate it again.
    fn equate_var(
        &mut self,
        universal_regions: &UniversalRegions<'tcx>,
        var: CanonicalVar,
        b_kind: Kind<'tcx>,
    ) -> RelateResult<'tcx, Kind<'tcx>> {
        debug!("equate_var(var={:?}, b_kind={:?})", var, b_kind);

        // We only encounter canonical variables when equating.
        assert_eq!(self.ambient_variance, ty::Variance::Invariant);

        // The canonical variable already had a value. Equate that
        // value with `b`.
        if let Some(a_kind) = self.canonical_var_values[var] {
            debug!("equate_var: a_kind={:?}", a_kind);

            // The values we extract from `canonical_var_values` have
            // been "instantiated" and hence the set of scopes we have
            // doesn't matter -- just to be sure, put an empty vector
            // in there.
            let old_a_scopes = mem::replace(&mut self.a_scopes, vec![]);
            let result = self.relate(&a_kind, &b_kind);
            self.a_scopes = old_a_scopes;

            debug!("equate_var: complete, result = {:?}", result);
            return result;
        }

        // Not yet. Capture the value from the RHS and carry on.
        let closed_kind =
            self.instantiate_traversed_binders(universal_regions, &self.b_scopes, b_kind);
        self.canonical_var_values[var] = Some(closed_kind);
        debug!(
            "equate_var: capturing value {:?}",
            self.canonical_var_values[var]
        );

        // FIXME -- technically, we should add some sort of
        // assertion that this value can be named in the universe
        // of the canonical variable. But in practice these
        // canonical variables only arise presently in cases where
        // they are in the root universe and the main typeck has
        // ensured there are no universe errors. So we just kind
        // of over look this right now.
        Ok(b_kind)
    }

    /// As we traverse types and pass through binders, we push the
    /// values for each of the regions bound by those binders onto
    /// `scopes`. This function goes through `kind` and replaces any
    /// references into those scopes with the corresponding free
    /// region. Thus the resulting value should have no escaping
    /// references to bound things and can be transported into other
    /// scopes.
    fn instantiate_traversed_binders(
        &self,
        universal_regions: &UniversalRegions<'tcx>,
        scopes: &[BoundRegionScope],
        kind: Kind<'tcx>,
    ) -> Kind<'tcx> {
        let k = kind.fold_with(&mut BoundReplacer {
            type_rel: self,
            first_free_index: ty::INNERMOST,
            universal_regions,
            scopes: scopes,
        });

        assert!(!k.has_escaping_regions());

        k
    }
}

impl<'cx, 'bccx, 'gcx, 'tcx> TypeRelation<'cx, 'gcx, 'tcx>
    for TypeRelating<'cx, 'bccx, 'gcx, 'tcx>
{
    fn tcx(&self) -> TyCtxt<'cx, 'gcx, 'tcx> {
        self.infcx.tcx
    }

    fn tag(&self) -> &'static str {
        "nll::subtype"
    }

    fn a_is_expected(&self) -> bool {
        true
    }

    fn relate_with_variance<T: Relate<'tcx>>(
        &mut self,
        variance: ty::Variance,
        a: &T,
        b: &T,
    ) -> RelateResult<'tcx, T> {
        debug!(
            "relate_with_variance(variance={:?}, a={:?}, b={:?})",
            variance, a, b
        );

        let old_ambient_variance = self.ambient_variance;
        self.ambient_variance = self.ambient_variance.xform(variance);

        debug!(
            "relate_with_variance: ambient_variance = {:?}",
            self.ambient_variance
        );

        let r = self.relate(a, b)?;

        self.ambient_variance = old_ambient_variance;

        debug!("relate_with_variance: r={:?}", r);

        Ok(r)
    }

    fn tys(&mut self, a: Ty<'tcx>, b: Ty<'tcx>) -> RelateResult<'tcx, Ty<'tcx>> {
        // Watch out for the case that we are matching a `?T` against the
        // right-hand side.
        if let ty::Infer(ty::CanonicalTy(var)) = a.sty {
            if let Some(&mut BorrowCheckContext {
                universal_regions, ..
            }) = self.borrowck_context
            {
                self.equate_var(universal_regions, var, b.into())?;
                Ok(a)
            } else {
                // if NLL is not enabled just ignore these variables
                // for now; in that case we're just doing a "sanity
                // check" anyway, and this only affects user-given
                // annotations like `let x: Vec<_> = ...` -- and then
                // only if the user uses type aliases to make a type
                // variable repeat more than once.
                Ok(a)
            }
        } else {
            debug!(
                "tys(a={:?}, b={:?}, variance={:?})",
                a, b, self.ambient_variance
            );

            relate::super_relate_tys(self, a, b)
        }
    }

    fn regions(
        &mut self,
        a: ty::Region<'tcx>,
        b: ty::Region<'tcx>,
    ) -> RelateResult<'tcx, ty::Region<'tcx>> {
        if let Some(&mut BorrowCheckContext {
            universal_regions, ..
        }) = self.borrowck_context
        {
            if let ty::ReCanonical(var) = a {
                self.equate_var(universal_regions, *var, b.into())?;
                return Ok(a);
            }

            debug!(
                "regions(a={:?}, b={:?}, variance={:?})",
                a, b, self.ambient_variance
            );

            let v_a =
                self.replace_bound_region(universal_regions, a, ty::INNERMOST, &self.a_scopes);
            let v_b =
                self.replace_bound_region(universal_regions, b, ty::INNERMOST, &self.b_scopes);

            debug!("regions: v_a = {:?}", v_a);
            debug!("regions: v_b = {:?}", v_b);

            if self.ambient_covariance() {
                // Covariance: a <= b. Hence, `b: a`.
                self.push_outlives(v_b, v_a);
            }

            if self.ambient_contravariance() {
                // Contravariant: b <= a. Hence, `a: b`.
                self.push_outlives(v_a, v_b);
            }
        }

        Ok(a)
    }

    fn binders<T>(
        &mut self,
        a: &ty::Binder<T>,
        b: &ty::Binder<T>,
    ) -> RelateResult<'tcx, ty::Binder<T>>
    where
        T: Relate<'tcx>,
    {
        // We want that
        //
        // ```
        // for<'a> fn(&'a u32) -> &'a u32 <:
        //   fn(&'b u32) -> &'b u32
        // ```
        //
        // but not
        //
        // ```
        // fn(&'a u32) -> &'a u32 <:
        //   for<'b> fn(&'b u32) -> &'b u32
        // ```
        //
        // We therefore proceed as follows:
        //
        // - Instantiate binders on `b` universally, yielding a universe U1.
        // - Instantiate binders on `a` existentially in U1.

        debug!(
            "binders({:?}: {:?}, ambient_variance={:?})",
            a, b, self.ambient_variance
        );

        if self.ambient_covariance() {
            // Covariance, so we want `for<..> A <: for<..> B` --
            // therefore we compare any instantiation of A (i.e., A
            // instantiated with existentials) against every
            // instantiation of B (i.e., B instantiated with
            // universals).

            let b_scope = self.create_scope(b, UniversallyQuantified(true));
            let a_scope = self.create_scope(a, UniversallyQuantified(false));

            debug!("binders: a_scope = {:?} (existential)", a_scope);
            debug!("binders: b_scope = {:?} (universal)", b_scope);

            self.b_scopes.push(b_scope);
            self.a_scopes.push(a_scope);

            // FIXME -- to be fully correct, we would set the ambient
            // variance to Covariant here. As is, we will sometimes
            // propagate down an ambient variance of Equal -- this in
            // turn causes us to report errors in some cases where
            // types perhaps *ought* to be equal. See the
            // `hr-fn-aau-eq-abu.rs` test for an example. Fixing this
            // though is a bit nontrivial: in particular, it would
            // require a more involved handling of canonical
            // variables, since we would no longer be able to rely on
            // having an `==` relationship for canonical variables.

            self.relate(a.skip_binder(), b.skip_binder())?;

            self.b_scopes.pop().unwrap();
            self.a_scopes.pop().unwrap();
        }

        if self.ambient_contravariance() {
            // Contravariance, so we want `for<..> A :> for<..> B`
            // -- therefore we compare every instantiation of A (i.e.,
            // A instantiated with universals) against any
            // instantiation of B (i.e., B instantiated with
            // existentials). Opposite of above.

            let a_scope = self.create_scope(a, UniversallyQuantified(true));
            let b_scope = self.create_scope(b, UniversallyQuantified(false));

            debug!("binders: a_scope = {:?} (universal)", a_scope);
            debug!("binders: b_scope = {:?} (existential)", b_scope);

            self.a_scopes.push(a_scope);
            self.b_scopes.push(b_scope);

            self.relate(a.skip_binder(), b.skip_binder())?;

            self.b_scopes.pop().unwrap();
            self.a_scopes.pop().unwrap();
        }

        Ok(a.clone())
    }
}

/// When we encounter a binder like `for<..> fn(..)`, we actually have
/// to walk the `fn` value to find all the values bound by the `for`
/// (these are not explicitly present in the ty representation right
/// now). This visitor handles that: it descends the type, tracking
/// binder depth, and finds late-bound regions targeting the
/// `for<..`>.  For each of those, it creates an entry in
/// `bound_region_scope`.
struct ScopeInstantiator<'cx, 'gcx: 'tcx, 'tcx: 'cx> {
    infcx: &'cx InferCtxt<'cx, 'gcx, 'tcx>,
    // The debruijn index of the scope we are instantiating.
    target_index: ty::DebruijnIndex,
    universally_quantified: UniversallyQuantified,
    bound_region_scope: &'cx mut BoundRegionScope,
}

impl<'cx, 'gcx, 'tcx> TypeVisitor<'tcx> for ScopeInstantiator<'cx, 'gcx, 'tcx> {
    fn visit_binder<T: TypeFoldable<'tcx>>(&mut self, t: &ty::Binder<T>) -> bool {
        self.target_index.shift_in(1);
        t.super_visit_with(self);
        self.target_index.shift_out(1);

        false
    }

    fn visit_region(&mut self, r: ty::Region<'tcx>) -> bool {
        let ScopeInstantiator {
            infcx,
            universally_quantified,
            ..
        } = *self;

        match r {
            ty::ReLateBound(debruijn, br) if *debruijn == self.target_index => {
                self.bound_region_scope.map.entry(*br).or_insert_with(|| {
                    let origin = if universally_quantified.0 {
                        NLLRegionVariableOrigin::BoundRegion(infcx.create_subuniverse())
                    } else {
                        NLLRegionVariableOrigin::Existential
                    };
                    infcx.next_nll_region_var(origin).to_region_vid()
                });
            }

            _ => {}
        }

        false
    }
}

/// When we encounter a binder like `for<..> fn(..)`, we actually have
/// to walk the `fn` value to find all the values bound by the `for`
/// (these are not explicitly present in the ty representation right
/// now). This visitor handles that: it descends the type, tracking
/// binder depth, and finds late-bound regions targeting the
/// `for<..`>.  For each of those, it creates an entry in
/// `bound_region_scope`.
struct BoundReplacer<'me, 'bccx: 'me, 'gcx: 'tcx, 'tcx: 'bccx> {
    type_rel: &'me TypeRelating<'me, 'bccx, 'gcx, 'tcx>,
    first_free_index: ty::DebruijnIndex,
    universal_regions: &'me UniversalRegions<'tcx>,
    scopes: &'me [BoundRegionScope],
}

impl TypeFolder<'gcx, 'tcx> for BoundReplacer<'me, 'bccx, 'gcx, 'tcx> {
    fn tcx(&self) -> TyCtxt<'_, 'gcx, 'tcx> {
        self.type_rel.tcx()
    }

    fn fold_binder<T: TypeFoldable<'tcx>>(&mut self, t: &ty::Binder<T>) -> ty::Binder<T> {
        self.first_free_index.shift_in(1);
        let result = t.super_fold_with(self);
        self.first_free_index.shift_out(1);
        result
    }

    fn fold_region(&mut self, r: ty::Region<'tcx>) -> ty::Region<'tcx> {
        let tcx = self.tcx();

        if let ty::ReLateBound(debruijn, _) = r {
            if *debruijn < self.first_free_index {
                return r;
            }
        }

        let region_vid = self.type_rel.replace_bound_region(
            self.universal_regions,
            r,
            self.first_free_index,
            self.scopes,
        );

        tcx.mk_region(ty::ReVar(region_vid))
    }
}
