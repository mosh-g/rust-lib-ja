// Copyright 2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

// aux-build:two_macros.rs

#![feature(macro_reexport)]

#[macro_use(macro_two)]
#[macro_reexport(no_way)] //~ ERROR reexported macro not found
extern crate two_macros;

pub fn main() {
    macro_two!();
}
