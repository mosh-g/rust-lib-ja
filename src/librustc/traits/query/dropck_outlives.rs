// Copyright 2014 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use infer::at::At;
use infer::canonical::{Canonical, Canonicalize, QueryResult};
use infer::InferOk;
use std::iter::FromIterator;
use traits::query::CanonicalTyGoal;
use ty::{self, Ty, TyCtxt};
use ty::subst::Kind;
use std::rc::Rc;

impl<'cx, 'gcx, 'tcx> At<'cx, 'gcx, 'tcx> {
    /// Given a type `ty` of some value being dropped, computes a set
    /// of "kinds" (types, regions) that must be outlive the execution
    /// of the destructor. These basically correspond to data that the
    /// destructor might access. This is used during regionck to
    /// impose "outlives" constraints on any lifetimes referenced
    /// within.
    ///
    /// The rules here are given by the "dropck" RFCs, notably [#1238]
    /// and [#1327]. This is a fixed-point computation, where we
    /// explore all the data that will be dropped (transitively) when
    /// a value of type `ty` is dropped. For each type T that will be
    /// dropped and which has a destructor, we must assume that all
    /// the types/regions of T are live during the destructor, unless
    /// they are marked with a special attribute (`#[may_dangle]`).
    ///
    /// [#1238]: https://github.com/rust-lang/rfcs/blob/master/text/1238-nonparametric-dropck.md
    /// [#1327]: https://github.com/rust-lang/rfcs/blob/master/text/1327-dropck-param-eyepatch.md
    pub fn dropck_outlives(&self, ty: Ty<'tcx>) -> InferOk<'tcx, Vec<Kind<'tcx>>> {
        debug!(
            "dropck_outlives(ty={:?}, param_env={:?})",
            ty, self.param_env,
        );

        let tcx = self.infcx.tcx;
        let gcx = tcx.global_tcx();
        let (c_ty, orig_values) = self.infcx.canonicalize_query(&self.param_env.and(ty));
        let span = self.cause.span;
        match &gcx.dropck_outlives(c_ty) {
            Ok(result) if result.is_proven() => {
                match self.infcx.instantiate_query_result(
                    self.cause,
                    self.param_env,
                    &orig_values,
                    result,
                ) {
                    Ok(InferOk {
                        value: DropckOutlivesResult { kinds, overflows },
                        obligations,
                    }) => {
                        for overflow_ty in overflows.into_iter().take(1) {
                            let mut err = struct_span_err!(
                                tcx.sess,
                                span,
                                E0320,
                                "overflow while adding drop-check rules for {}",
                                self.infcx.resolve_type_vars_if_possible(&ty),
                            );
                            err.note(&format!("overflowed on {}", overflow_ty));
                            err.emit();
                        }

                        return InferOk {
                            value: kinds,
                            obligations,
                        };
                    }

                    Err(_) => { /* fallthrough to error-handling code below */ }
                }
            }

            _ => { /* fallthrough to error-handling code below */ }
        }

        // Errors and ambiuity in dropck occur in two cases:
        // - unresolved inference variables at the end of typeck
        // - non well-formed types where projections cannot be resolved
        // Either of these should hvae created an error before.
        tcx.sess
            .delay_span_bug(span, "dtorck encountered internal error");
        return InferOk {
            value: vec![],
            obligations: vec![],
        };
    }
}

#[derive(Clone, Debug)]
pub struct DropckOutlivesResult<'tcx> {
    pub kinds: Vec<Kind<'tcx>>,
    pub overflows: Vec<Ty<'tcx>>,
}

/// A set of constraints that need to be satisfied in order for
/// a type to be valid for destruction.
#[derive(Clone, Debug)]
pub struct DtorckConstraint<'tcx> {
    /// Types that are required to be alive in order for this
    /// type to be valid for destruction.
    pub outlives: Vec<ty::subst::Kind<'tcx>>,

    /// Types that could not be resolved: projections and params.
    pub dtorck_types: Vec<Ty<'tcx>>,

    /// If, during the computation of the dtorck constraint, we
    /// overflow, that gets recorded here. The caller is expected to
    /// report an error.
    pub overflows: Vec<Ty<'tcx>>,
}

impl<'tcx> DtorckConstraint<'tcx> {
    pub fn empty() -> DtorckConstraint<'tcx> {
        DtorckConstraint {
            outlives: vec![],
            dtorck_types: vec![],
            overflows: vec![],
        }
    }
}

impl<'tcx> FromIterator<DtorckConstraint<'tcx>> for DtorckConstraint<'tcx> {
    fn from_iter<I: IntoIterator<Item = DtorckConstraint<'tcx>>>(iter: I) -> Self {
        let mut result = Self::empty();

        for DtorckConstraint {
            outlives,
            dtorck_types,
            overflows,
        } in iter
        {
            result.outlives.extend(outlives);
            result.dtorck_types.extend(dtorck_types);
            result.overflows.extend(overflows);
        }

        result
    }
}
impl<'gcx: 'tcx, 'tcx> Canonicalize<'gcx, 'tcx> for ty::ParamEnvAnd<'tcx, Ty<'tcx>> {
    type Canonicalized = CanonicalTyGoal<'gcx>;

    fn intern(
        _gcx: TyCtxt<'_, 'gcx, 'gcx>,
        value: Canonical<'gcx, Self::Lifted>,
    ) -> Self::Canonicalized {
        value
    }
}

BraceStructTypeFoldableImpl! {
    impl<'tcx> TypeFoldable<'tcx> for DropckOutlivesResult<'tcx> {
        kinds, overflows
    }
}

BraceStructLiftImpl! {
    impl<'a, 'tcx> Lift<'tcx> for DropckOutlivesResult<'a> {
        type Lifted = DropckOutlivesResult<'tcx>;
        kinds, overflows
    }
}

impl_stable_hash_for!(struct DropckOutlivesResult<'tcx> {
    kinds, overflows
});

impl<'gcx: 'tcx, 'tcx> Canonicalize<'gcx, 'tcx> for QueryResult<'tcx, DropckOutlivesResult<'tcx>> {
    // we ought to intern this, but I'm too lazy just now
    type Canonicalized = Rc<Canonical<'gcx, QueryResult<'gcx, DropckOutlivesResult<'gcx>>>>;

    fn intern(
        _gcx: TyCtxt<'_, 'gcx, 'gcx>,
        value: Canonical<'gcx, Self::Lifted>,
    ) -> Self::Canonicalized {
        Rc::new(value)
    }
}

impl_stable_hash_for!(struct DtorckConstraint<'tcx> {
    outlives,
    dtorck_types,
    overflows
});
