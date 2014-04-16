// Copyright 2012 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

fn foo(x: &str) -> u8 {
    x[0]
}

pub fn main() {
    let p = "hello".to_owned();
    let r = foo(p);
    assert_eq!(r, 'h' as u8);

    let p = "hello".to_owned();
    let r = foo(p);
    assert_eq!(r, 'h' as u8);
}
