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
#![allow(non_snake_case)]

// pretty-expanded FIXME #23616

#![feature(box_syntax)]

struct Font {
    fontbuf: usize,
    cairo_font: usize,
    font_dtor: usize,

}

impl Drop for Font {
    fn drop(&mut self) {}
}

fn Font() -> Font {
    Font {
        fontbuf: 0,
        cairo_font: 0,
        font_dtor: 0
    }
}

pub fn main() {
    let _f: Box<_> = box Font();
}
