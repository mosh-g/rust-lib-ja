// Copyright 2016 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use infer::canonical::{Canonical, Canonicalized, CanonicalizedQueryResult, QueryRegionConstraint,
                       QueryResult};
use infer::{InferCtxt, InferOk};
use smallvec::SmallVec;
use std::fmt;
use std::rc::Rc;
use traits::query::Fallible;
use traits::ObligationCause;
use ty::fold::TypeFoldable;
use ty::{Lift, ParamEnvAnd, TyCtxt};

pub mod custom;
pub mod eq;
pub mod implied_outlives_bounds;
pub mod normalize;
pub mod outlives;
pub mod prove_predicate;
use self::prove_predicate::ProvePredicate;
pub mod subtype;

/// "Type ops" are used in NLL to perform some particular action and
/// extract out the resulting region constraints (or an error if it
/// cannot be completed).
pub trait TypeOp<'gcx, 'tcx>: Sized + fmt::Debug {
    type Output;

    /// Processes the operation and all resulting obligations,
    /// returning the final result along with any region constraints
    /// (they will be given over to the NLL region solver).
    fn fully_perform(
        self,
        infcx: &InferCtxt<'_, 'gcx, 'tcx>,
    ) -> Fallible<(Self::Output, Option<Rc<Vec<QueryRegionConstraint<'tcx>>>>)>;
}

/// "Query type ops" are type ops that are implemented using a
/// [canonical query][c]. The `Self` type here contains the kernel of
/// information needed to do the operation -- `TypeOp` is actually
/// implemented for `ParamEnvAnd<Self>`, since we always need to bring
/// along a parameter environment as well. For query type-ops, we will
/// first canonicalize the key and then invoke the query on the tcx,
/// which produces the resulting query region constraints.
///
/// [c]: https://rust-lang-nursery.github.io/rustc-guide/traits/canonicalization.html
pub trait QueryTypeOp<'gcx: 'tcx, 'tcx>:
    fmt::Debug + Sized + TypeFoldable<'tcx> + Lift<'gcx>
{
    type QueryResult: TypeFoldable<'tcx> + Lift<'gcx>;

    /// Give query the option for a simple fast path that never
    /// actually hits the tcx cache lookup etc. Return `Some(r)` with
    /// a final result or `None` to do the full path.
    fn try_fast_path(
        tcx: TyCtxt<'_, 'gcx, 'tcx>,
        key: &ParamEnvAnd<'tcx, Self>,
    ) -> Option<Self::QueryResult>;

    /// Performs the actual query with the canonicalized key -- the
    /// real work happens here. This method is not given an `infcx`
    /// because it shouldn't need one -- and if it had access to one,
    /// it might do things like invoke `sub_regions`, which would be
    /// bad, because it would create subregion relationships that are
    /// not captured in the return value.
    fn perform_query(
        tcx: TyCtxt<'_, 'gcx, 'tcx>,
        canonicalized: Canonicalized<'gcx, ParamEnvAnd<'tcx, Self>>,
    ) -> Fallible<CanonicalizedQueryResult<'gcx, Self::QueryResult>>;

    /// Casts a lifted query result (which is in the gcx lifetime)
    /// into the tcx lifetime. This is always just an identity cast,
    /// but the generic code doesn't realize it -- put another way, in
    /// the generic code, we have a `Lifted<'gcx, Self::QueryResult>`
    /// and we want to convert that to a `Self::QueryResult`. This is
    /// not a priori valid, so we can't do it -- but in practice, it
    /// is always a no-op (e.g., the lifted form of a type,
    /// `Ty<'gcx>`, is a subtype of `Ty<'tcx>`). So we have to push
    /// the operation into the impls that know more specifically what
    /// `QueryResult` is. This operation would (maybe) be nicer with
    /// something like HKTs or GATs, since then we could make
    /// `QueryResult` parametric and `'gcx` and `'tcx` etc.
    fn shrink_to_tcx_lifetime(
        lifted_query_result: &'a CanonicalizedQueryResult<'gcx, Self::QueryResult>,
    ) -> &'a Canonical<'tcx, QueryResult<'tcx, Self::QueryResult>>;

    fn fully_perform_into(
        query_key: ParamEnvAnd<'tcx, Self>,
        infcx: &InferCtxt<'_, 'gcx, 'tcx>,
        output_query_region_constraints: &mut Vec<QueryRegionConstraint<'tcx>>,
    ) -> Fallible<Self::QueryResult> {
        if let Some(result) = QueryTypeOp::try_fast_path(infcx.tcx, &query_key) {
            return Ok(result);
        }

        // FIXME(#33684) -- We need to use
        // `canonicalize_hr_query_hack` here because of things
        // like the subtype query, which go awry around
        // `'static` otherwise.
        let mut canonical_var_values = SmallVec::new();
        let canonical_self =
            infcx.canonicalize_hr_query_hack(&query_key, &mut canonical_var_values);
        let canonical_result = Self::perform_query(infcx.tcx, canonical_self)?;
        let canonical_result = Self::shrink_to_tcx_lifetime(&canonical_result);

        let param_env = query_key.param_env;

        let InferOk { value, obligations } = infcx
            .instantiate_nll_query_result_and_region_obligations(
                &ObligationCause::dummy(),
                param_env,
                &canonical_var_values,
                canonical_result,
                output_query_region_constraints,
            )?;

        // Typically, instantiating NLL query results does not
        // create obligations. However, in some cases there
        // are unresolved type variables, and unify them *can*
        // create obligations. In that case, we have to go
        // fulfill them. We do this via a (recursive) query.
        for obligation in obligations {
            let () = ProvePredicate::fully_perform_into(
                obligation
                    .param_env
                    .and(ProvePredicate::new(obligation.predicate)),
                infcx,
                output_query_region_constraints,
            )?;
        }

        Ok(value)
    }
}

impl<'gcx: 'tcx, 'tcx, Q> TypeOp<'gcx, 'tcx> for ParamEnvAnd<'tcx, Q>
where
    Q: QueryTypeOp<'gcx, 'tcx>,
{
    type Output = Q::QueryResult;

    fn fully_perform(
        self,
        infcx: &InferCtxt<'_, 'gcx, 'tcx>,
    ) -> Fallible<(Self::Output, Option<Rc<Vec<QueryRegionConstraint<'tcx>>>>)> {
        let mut qrc = vec![];
        let r = Q::fully_perform_into(self, infcx, &mut qrc)?;

        // Promote the final query-region-constraints into a
        // (optional) ref-counted vector:
        let opt_qrc = if qrc.is_empty() {
            None
        } else {
            Some(Rc::new(qrc))
        };

        Ok((r, opt_qrc))
    }
}
