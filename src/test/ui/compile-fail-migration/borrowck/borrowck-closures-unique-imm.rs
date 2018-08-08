// Copyright 2014 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

// ignore-compare-mode-nll

struct Foo {
    x: isize,
}

pub fn main() {
    let mut this = &mut Foo {
        x: 1,
    };
    let mut r = || {
        let p = &this.x;
        &mut this.x; //~ ERROR cannot borrow
    };
    r()
}
