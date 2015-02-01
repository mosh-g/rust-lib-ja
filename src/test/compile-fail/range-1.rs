// Copyright 2014 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

// Test range syntax - type errors.

pub fn main() {
    // Mixed types.
    let _ = 0u32..10i32;
    //~^ ERROR start and end of range have incompatible types

    // Float => does not implement iterator.
    for i in 0f32..42f32 {}
    //~^ ERROR the trait `core::num::Int` is not implemented for the type `f32`

    // Unsized type.
    let arr: &[_] = &[1u32, 2, 3];
    let range = *arr..;
    //~^ ERROR the trait `core::marker::Sized` is not implemented
}
