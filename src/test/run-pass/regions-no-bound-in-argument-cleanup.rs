// Copyright 2014 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

#![feature(associated_types, unsafe_destructor)]

pub struct Foo<T>;

impl<T> Iterator for Foo<T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        None
    }
}

#[unsafe_destructor]
impl<T> Drop for Foo<T> {
    fn drop(&mut self) {
        self.next();
    }
}

pub fn foo<'a>(_: Foo<&'a ()>) {}

pub fn main() {}
