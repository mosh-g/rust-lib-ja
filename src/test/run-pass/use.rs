
// Copyright 2012-2014 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

#![allow(unused_imports)]
#![feature(start, no_std)]
#![no_std]

extern crate std;
extern crate "std" as zed;

use std::str;
use zed::str as x;
mod baz {
    pub use std::str as x;
}

#[start]
pub fn start(_: int, _: *const *const u8) -> int { 0 }
