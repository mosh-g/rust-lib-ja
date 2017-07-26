// Copyright 2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.


// FIXME: remove the next line when https://github.com/rust-lang/rust/issues/43358 is resolved
// compile-flags: -Zmir-opt-level=0

// Check that you can cast between different pointers to trait objects
// whose vtable have the same kind (both lengths, or both trait pointers).

trait Foo<T> {
    fn foo(&self, _: T) -> u32 { 42 }
}

trait Bar {
    fn bar(&self) { println!("Bar!"); }
}

impl<T> Foo<T> for () {}
impl Foo<u32> for u32 { fn foo(&self, _: u32) -> u32 { self+43 } }
impl Bar for () {}

unsafe fn round_trip_and_call<'a>(t: *const (Foo<u32>+'a)) -> u32 {
    let foo_e : *const Foo<u16> = t as *const _;
    let r_1 = foo_e as *mut Foo<u32>;

    (&*r_1).foo(0)
}

#[repr(C)]
struct FooS<T:?Sized>(T);
#[repr(C)]
struct BarS<T:?Sized>(T);

fn foo_to_bar<T:?Sized>(u: *const FooS<T>) -> *const BarS<T> {
    u as *const BarS<T>
}

fn main() {
    let x = 4u32;
    let y : &Foo<u32> = &x;
    let fl = unsafe { round_trip_and_call(y as *const Foo<u32>) };
    assert_eq!(fl, (43+4));

    let s = FooS([0,1,2]);
    let u: &FooS<[u32]> = &s;
    let u: *const FooS<[u32]> = u;
    let bar_ref : *const BarS<[u32]> = foo_to_bar(u);
    let z : &BarS<[u32]> = unsafe{&*bar_ref};
    assert_eq!(&z.0, &[0,1,2]);
    // If validation fails here, that's likely because an immutable suspension is recovered mutably.
}
