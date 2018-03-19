// Copyright 2018 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

#![feature(deprecated)]

// @has deprecated_future/struct.S.html '//*[@class="stab deprecated"]' \
//      'This will be deprecated in 99.99.99: effectively never'
#[deprecated(since = "99.99.99", note = "effectively never")]
pub struct S;
