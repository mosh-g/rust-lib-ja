// Copyright 2018 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

// compile-flags: --edition 2018
// aux-build:removing-extern-crate.rs
// run-rustfix
// compile-pass

#![warn(rust_2018_idioms)]
#![allow(unused_imports)]

extern crate std as foo;
extern crate core;

mod another {
    extern crate std as foo;
    extern crate std;
}

fn main() {}
