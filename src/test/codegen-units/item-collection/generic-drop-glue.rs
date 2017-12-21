// Copyright 2012-2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

// ignore-tidy-linelength
// compile-flags:-Zprint-trans-items=eager
// compile-flags:-Zinline-in-all-cgus

#![deny(dead_code)]

struct StructWithDrop<T1, T2> {
    x: T1,
    y: T2,
}

impl<T1, T2> Drop for StructWithDrop<T1, T2> {
    fn drop(&mut self) {}
}

struct StructNoDrop<T1, T2> {
    x: T1,
    y: T2,
}

enum EnumWithDrop<T1, T2> {
    A(T1),
    B(T2)
}

impl<T1, T2> Drop for EnumWithDrop<T1, T2> {
    fn drop(&mut self) {}
}

enum EnumNoDrop<T1, T2> {
    A(T1),
    B(T2)
}


struct NonGenericNoDrop(i32);

struct NonGenericWithDrop(i32);
//~ TRANS_ITEM fn core::ptr[0]::drop_in_place[0]<generic_drop_glue::NonGenericWithDrop[0]> @@ generic_drop_glue0[Internal]

impl Drop for NonGenericWithDrop {
    //~ TRANS_ITEM fn generic_drop_glue::{{impl}}[2]::drop[0]
    fn drop(&mut self) {}
}

//~ TRANS_ITEM fn alloc::allocator[0]::{{impl}}[0]::align[0] @@ generic_drop_glue0[Internal]
//~ TRANS_ITEM fn alloc::allocator[0]::{{impl}}[0]::from_size_align_unchecked[0] @@ generic_drop_glue0[Internal]
//~ TRANS_ITEM fn alloc::allocator[0]::{{impl}}[0]::size[0] @@ generic_drop_glue0[Internal]
//~ TRANS_ITEM fn alloc::heap[0]::box_free[0]<core::any[0]::Any[0]> @@ generic_drop_glue0[Internal]
//~ TRANS_ITEM fn alloc::heap[0]::{{impl}}[0]::dealloc[0] @@ generic_drop_glue0[Internal]
//~ TRANS_ITEM fn core::mem[0]::uninitialized[0]<std::rt[0]::lang_start[0]::{{closure}}[0]<()>> @@ generic_drop_glue0[Internal]
//~ TRANS_ITEM fn core::ptr[0]::drop_in_place[0]<alloc::boxed[0]::Box[0]<core::any[0]::Any[0]>> @@ generic_drop_glue0[Internal]
//~ TRANS_ITEM fn core::ptr[0]::drop_in_place[0]<core::any[0]::Any[0]> @@ generic_drop_glue0[Internal]
//~ TRANS_ITEM fn core::ptr[0]::drop_in_place[0]<core::result[0]::Result[0]<i32, alloc::boxed[0]::Box[0]<core::any[0]::Any[0]>>> @@ generic_drop_glue0[Internal]
//~ TRANS_ITEM fn core::ptr[0]::read[0]<std::rt[0]::lang_start[0]::{{closure}}[0]<()>> @@ generic_drop_glue0[Internal]
//~ TRANS_ITEM fn core::ptr[0]::write[0]<i32> @@ generic_drop_glue0[Internal]
//~ TRANS_ITEM fn core::result[0]::{{impl}}[0]::unwrap_or[0]<i32, alloc::boxed[0]::Box[0]<core::any[0]::Any[0]>> @@ generic_drop_glue0[Internal]
//~ TRANS_ITEM fn std::panic[0]::catch_unwind[0]<std::rt[0]::lang_start[0]::{{closure}}[0]<()>, i32> @@ generic_drop_glue0[Internal]
//~ TRANS_ITEM fn std::panicking[0]::try[0]::do_call[0]<std::rt[0]::lang_start[0]::{{closure}}[0]<()>, i32> @@ generic_drop_glue0[Internal]
//~ TRANS_ITEM fn std::panicking[0]::try[0]<i32, std::rt[0]::lang_start[0]::{{closure}}[0]<()>> @@ generic_drop_glue0[Internal]
//~ TRANS_ITEM fn std::rt[0]::lang_start[0]::{{closure}}[0]::{{closure}}[0]<(), i32, extern "rust-call" fn(()) -> i32, fn()> @@ generic_drop_glue0[Internal]
//~ TRANS_ITEM fn std::rt[0]::lang_start[0]::{{closure}}[0]<(), i32, extern "rust-call" fn(()) -> i32, &fn()> @@ generic_drop_glue0[Internal]
//~ TRANS_ITEM fn std::rt[0]::lang_start[0]<()> @@ generic_drop_glue0[External]
//~ TRANS_ITEM fn std::sys_common[0]::backtrace[0]::__rust_begin_short_backtrace[0]<std::rt[0]::lang_start[0]::{{closure}}[0]::{{closure}}[0]<()>, i32> @@ generic_drop_glue0[Internal]
//~ TRANS_ITEM fn generic_drop_glue::main[0]
fn main() {
    //~ TRANS_ITEM fn core::ptr[0]::drop_in_place[0]<generic_drop_glue::StructWithDrop[0]<i8, char>> @@ generic_drop_glue0[Internal]
    //~ TRANS_ITEM fn generic_drop_glue::{{impl}}[0]::drop[0]<i8, char>
    let _ = StructWithDrop { x: 0i8, y: 'a' }.x;

    //~ TRANS_ITEM fn core::ptr[0]::drop_in_place[0]<generic_drop_glue::StructWithDrop[0]<&str, generic_drop_glue::NonGenericNoDrop[0]>> @@ generic_drop_glue0[Internal]
    //~ TRANS_ITEM fn generic_drop_glue::{{impl}}[0]::drop[0]<&str, generic_drop_glue::NonGenericNoDrop[0]>
    let _ = StructWithDrop { x: "&str", y: NonGenericNoDrop(0) }.y;

    // Should produce no drop glue
    let _ = StructNoDrop { x: 'a', y: 0u32 }.x;

    // This is supposed to generate drop-glue because it contains a field that
    // needs to be dropped.
    //~ TRANS_ITEM fn core::ptr[0]::drop_in_place[0]<generic_drop_glue::StructNoDrop[0]<generic_drop_glue::NonGenericWithDrop[0], f64>> @@ generic_drop_glue0[Internal]
    let _ = StructNoDrop { x: NonGenericWithDrop(0), y: 0f64 }.y;

    //~ TRANS_ITEM fn core::ptr[0]::drop_in_place[0]<generic_drop_glue::EnumWithDrop[0]<i32, i64>> @@ generic_drop_glue0[Internal]
    //~ TRANS_ITEM fn generic_drop_glue::{{impl}}[1]::drop[0]<i32, i64>
    let _ = match EnumWithDrop::A::<i32, i64>(0) {
        EnumWithDrop::A(x) => x,
        EnumWithDrop::B(x) => x as i32
    };

    //~TRANS_ITEM fn core::ptr[0]::drop_in_place[0]<generic_drop_glue::EnumWithDrop[0]<f64, f32>> @@ generic_drop_glue0[Internal]
    //~ TRANS_ITEM fn generic_drop_glue::{{impl}}[1]::drop[0]<f64, f32>
    let _ = match EnumWithDrop::B::<f64, f32>(1.0) {
        EnumWithDrop::A(x) => x,
        EnumWithDrop::B(x) => x as f64
    };

    let _ = match EnumNoDrop::A::<i32, i64>(0) {
        EnumNoDrop::A(x) => x,
        EnumNoDrop::B(x) => x as i32
    };

    let _ = match EnumNoDrop::B::<f64, f32>(1.0) {
        EnumNoDrop::A(x) => x,
        EnumNoDrop::B(x) => x as f64
    };
}
