// Copyright 2017 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

#![warn(anonymous_parameters)]
// Test for the anonymous_parameters deprecation lint (RFC 1685)

// compile-pass
// edition:2015
// run-rustfix

trait T {
    fn foo(i32); //~ WARNING anonymous parameters are deprecated
                 //~| WARNING hard error

    fn bar_with_default_impl(String, String) {}
    //~^ WARNING anonymous parameters are deprecated
    //~| WARNING hard error
    //~| WARNING anonymous parameters are deprecated
    //~| WARNING hard error
}

fn main() {}
