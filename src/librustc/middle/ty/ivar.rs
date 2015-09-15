// Copyright 2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use middle::ty::{Ty, TyS};

use rustc_data_structures::ivar;

use std::fmt;
use std::marker::PhantomData;
use core::nonzero::NonZero;

/// An IVar that contains a Ty. 'lt is a (reverse-variant) upper bound
/// on the lifetime of the IVar. This is required because of variance
/// problems: the IVar needs to be variant with respect to 'tcx (so
/// it can be referred to from Ty) but can only be modified if its
/// lifetime is exactly 'tcx.
///
/// Safety invariants:
///     (A) self.0, if fulfilled, is a valid Ty<'tcx>
///     (B) no aliases to this value with a 'tcx longer than this
///         value's 'lt exist
///
/// NonZero is used rather than Unique because Unique isn't Copy.
pub struct TyIVar<'tcx, 'lt: 'tcx>(ivar::Ivar<NonZero<*const TyS<'static>>>,
                                   PhantomData<fn(TyS<'lt>)->TyS<'tcx>>);

impl<'tcx, 'lt> TyIVar<'tcx, 'lt> {
    #[inline]
    pub fn new() -> Self {
        // Invariant (A) satisfied because the IVar is unfulfilled
        // Invariant (B) because 'lt : 'tcx
        TyIVar(ivar::Ivar::new(), PhantomData)
    }

    #[inline]
    pub fn get(&self) -> Option<Ty<'tcx>> {
        match self.0.get() {
            None => None,
            // valid because of invariant (A)
            Some(v) => Some(unsafe { &*(*v as *const TyS<'tcx>) })
        }
    }
    #[inline]
    pub fn unwrap(&self) -> Ty<'tcx> {
        self.get().unwrap()
    }

    pub fn fulfill(&self, value: Ty<'lt>) {
        // Invariant (A) is fulfilled, because by (B), every alias
        // of this has a 'tcx longer than 'lt.
        let value: *const TyS<'lt> = value;
        // FIXME(27214): unneeded [as *const ()]
        let value = value as *const () as *const TyS<'static>;
        self.0.fulfill(unsafe { NonZero::new(value) })
    }
}

impl<'tcx, 'lt> fmt::Debug for TyIVar<'tcx, 'lt> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.get() {
            Some(val) => write!(f, "TyIVar({:?})", val),
            None => f.write_str("TyIVar(<unfulfilled>)")
        }
    }
}
