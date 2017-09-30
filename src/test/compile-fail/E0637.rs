// Copyright 2017 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.
#![feature(underscore_lifetimes)]

struct Foo<'a: '_>(&'a u8); //~ ERROR invalid lifetime bound name: `'_`
fn foo<'a: '_>(_: &'a u8) {} //~ ERROR invalid lifetime bound name: `'_`

struct Bar<'a>(&'a u8);
impl<'a: '_> Bar<'a> { //~ ERROR invalid lifetime bound name: `'_`
  fn bar() {}
}

fn main() {}
