// Copyright 2018 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//https://github.com/rust-lang/rust/issues/31364

const fn a() -> usize { b() }
const fn b() -> usize { a() }
const ARR: [i32; a()] = [5; 6]; //~ ERROR could not evaluate constant expression

fn main(){}
