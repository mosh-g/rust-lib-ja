// Copyright 2018 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

// run-pass

#![feature(generators)]

fn main() {
    static || {
        loop {
            // Test that `opt` is not live across the yield, even when borrowed in a loop
            // See https://github.com/rust-lang/rust/issues/52792
            let opt = {
                yield;
                true
            };
            &opt;
        }
    };
}
