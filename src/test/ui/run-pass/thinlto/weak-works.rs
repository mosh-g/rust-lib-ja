// Copyright 2017 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

// run-pass

// compile-flags: -C codegen-units=8 -Z thinlto
// ignore-windows

#![feature(linkage)]

pub mod foo {
    #[linkage = "weak"]
    #[no_mangle]
    pub extern "C" fn FOO() -> i32 {
        0
    }
}

mod bar {
    extern "C" {
        fn FOO() -> i32;
    }

    pub fn bar() -> i32 {
        unsafe { FOO() }
    }
}

fn main() {
    bar::bar();
}
