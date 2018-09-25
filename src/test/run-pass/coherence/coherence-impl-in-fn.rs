// Copyright 2012 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

// run-pass
#![allow(dead_code)]
#![allow(non_camel_case_types)]

pub fn main() {
    #[derive(Copy, Clone)]
    enum x { foo }
    impl ::std::cmp::PartialEq for x {
        fn eq(&self, other: &x) -> bool {
            (*self) as isize == (*other) as isize
        }
        fn ne(&self, other: &x) -> bool { !(*self).eq(other) }
    }
}
