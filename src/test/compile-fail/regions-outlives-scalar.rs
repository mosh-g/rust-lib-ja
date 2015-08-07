// Copyright 2014 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

// Test that scalar values outlive all regions.
// Rule OutlivesScalar from RFC 1214.

#![feature(rustc_attrs)]
#![allow(dead_code)]

struct Foo<'a> {
    x: &'a i32,
    y: &'static i32
}

#[rustc_error]
fn main() { } //~ ERROR compilation successful
