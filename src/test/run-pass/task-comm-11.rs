// Copyright 2012 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

// xfail-fast
#[legacy_modes];

extern mod std;

fn start(c: pipes::Chan<pipes::Chan<int>>) {
    let (p, ch) = pipes::stream();
    c.send(ch);
}

pub fn main() {
    let (p, ch) = pipes::stream();
    let child = task::spawn(|| start(ch) );
    let c = p.recv();
}
