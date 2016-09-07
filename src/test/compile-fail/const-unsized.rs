// Copyright 2016 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use std::fmt::Debug;

const CONST_0: Debug+Sync = *(&0 as &(Debug+Sync));
//~^ ERROR `std::fmt::Debug + Sync + 'static: std::marker::Sized` is not satisfied
//~| NOTE `std::fmt::Debug + Sync + 'static: std::marker::Sized` not satisfied
//~| NOTE does not have a constant size known at compile-time
//~| NOTE constant expressions must have a statically known size
//~| WARNING This needs a `'static` lifetime or the `static_in_const` feature

const CONST_FOO: str = *"foo";
//~^ ERROR `str: std::marker::Sized` is not satisfied
//~| NOTE `str: std::marker::Sized` not satisfied
//~| NOTE does not have a constant size known at compile-time
//~| NOTE constant expressions must have a statically known size

static STATIC_1: Debug+Sync = *(&1 as &(Debug+Sync));
//~^ ERROR `std::fmt::Debug + Sync + 'static: std::marker::Sized` is not satisfied
//~| NOTE `std::fmt::Debug + Sync + 'static: std::marker::Sized` not satisfied
//~| NOTE does not have a constant size known at compile-time
//~| NOTE constant expressions must have a statically known size
//~| WARNING This needs a `'static` lifetime or the `static_in_const` feature

static STATIC_BAR: str = *"bar";
//~^ ERROR `str: std::marker::Sized` is not satisfied
//~| NOTE `str: std::marker::Sized` not satisfied
//~| NOTE does not have a constant size known at compile-time
//~| NOTE constant expressions must have a statically known size

fn main() {
    println!("{:?} {:?} {:?} {:?}", &CONST_0, &CONST_FOO, &STATIC_1, &STATIC_BAR);
}
