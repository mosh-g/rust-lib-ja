// Copyright 2014-2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

#![unstable(feature = "allocator_api",
            reason = "the precise API and guarantees it provides may be tweaked \
                      slightly, especially to possibly take into account the \
                      types being stored to make room for a future \
                      tracing garbage collector",
            issue = "32838")]

use core::intrinsics::{min_align_of_val, size_of_val};
use core::usize;

#[doc(inline)]
pub use core::alloc::*;

#[cfg(stage0)]
extern "Rust" {
    #[allocator]
    #[rustc_allocator_nounwind]
    fn __rust_alloc(size: usize, align: usize, err: *mut u8) -> *mut u8;
    #[rustc_allocator_nounwind]
    fn __rust_dealloc(ptr: *mut u8, size: usize, align: usize);
    #[rustc_allocator_nounwind]
    fn __rust_realloc(ptr: *mut u8,
                      old_size: usize,
                      old_align: usize,
                      new_size: usize,
                      new_align: usize,
                      err: *mut u8) -> *mut u8;
    #[rustc_allocator_nounwind]
    fn __rust_alloc_zeroed(size: usize, align: usize, err: *mut u8) -> *mut u8;
}

#[cfg(not(stage0))]
extern "Rust" {
    #[allocator]
    #[rustc_allocator_nounwind]
    fn __rust_alloc(size: usize, align: usize) -> *mut u8;
    #[rustc_allocator_nounwind]
    fn __rust_dealloc(ptr: *mut u8, size: usize, align: usize);
    #[rustc_allocator_nounwind]
    fn __rust_realloc(ptr: *mut u8,
                      old_size: usize,
                      align: usize,
                      new_size: usize) -> *mut u8;
    #[rustc_allocator_nounwind]
    fn __rust_alloc_zeroed(size: usize, align: usize) -> *mut u8;
}

#[derive(Copy, Clone, Default, Debug)]
pub struct Global;

#[unstable(feature = "allocator_api", issue = "32838")]
#[rustc_deprecated(since = "1.27.0", reason = "type renamed to `Global`")]
pub type Heap = Global;

#[unstable(feature = "allocator_api", issue = "32838")]
#[rustc_deprecated(since = "1.27.0", reason = "type renamed to `Global`")]
#[allow(non_upper_case_globals)]
pub const Heap: Global = Global;

unsafe impl Alloc for Global {
    #[inline]
    unsafe fn alloc(&mut self, layout: Layout) -> Result<*mut u8, AllocErr> {
        #[cfg(not(stage0))]
        let ptr = __rust_alloc(layout.size(), layout.align());
        #[cfg(stage0)]
        let ptr = __rust_alloc(layout.size(), layout.align(), &mut 0);

        if !ptr.is_null() {
            Ok(ptr)
        } else {
            Err(AllocErr)
        }
    }

    #[inline]
    unsafe fn dealloc(&mut self, ptr: *mut u8, layout: Layout) {
        __rust_dealloc(ptr, layout.size(), layout.align())
    }

    #[inline]
    unsafe fn realloc(&mut self,
                      ptr: *mut u8,
                      layout: Layout,
                      new_layout: Layout)
                      -> Result<*mut u8, AllocErr>
    {
        if layout.align() == new_layout.align() {
            #[cfg(not(stage0))]
            let ptr = __rust_realloc(ptr, layout.size(), layout.align(), new_layout.size());
            #[cfg(stage0)]
            let ptr = __rust_realloc(ptr, layout.size(), layout.align(),
                                     new_layout.size(), new_layout.align(), &mut 0);

            if !ptr.is_null() {
                Ok(ptr)
            } else {
                Err(AllocErr)
            }
        } else {
            Err(AllocErr)
        }
    }

    #[inline]
    unsafe fn alloc_zeroed(&mut self, layout: Layout) -> Result<*mut u8, AllocErr> {
        #[cfg(not(stage0))]
        let ptr = __rust_alloc_zeroed(layout.size(), layout.align());
        #[cfg(stage0)]
        let ptr = __rust_alloc_zeroed(layout.size(), layout.align(), &mut 0);

        if !ptr.is_null() {
            Ok(ptr)
        } else {
            Err(AllocErr)
        }
    }
}

/// The allocator for unique pointers.
// This function must not unwind. If it does, MIR trans will fail.
#[cfg(not(test))]
#[lang = "exchange_malloc"]
#[inline]
unsafe fn exchange_malloc(size: usize, align: usize) -> *mut u8 {
    if size == 0 {
        align as *mut u8
    } else {
        let layout = Layout::from_size_align_unchecked(size, align);
        Global.alloc(layout).unwrap_or_else(|_| {
            Global.oom()
        })
    }
}

#[cfg_attr(not(test), lang = "box_free")]
#[inline]
pub(crate) unsafe fn box_free<T: ?Sized>(ptr: *mut T) {
    let size = size_of_val(&*ptr);
    let align = min_align_of_val(&*ptr);
    // We do not allocate for Box<T> when T is ZST, so deallocation is also not necessary.
    if size != 0 {
        let layout = Layout::from_size_align_unchecked(size, align);
        Global.dealloc(ptr as *mut u8, layout);
    }
}

#[cfg(test)]
mod tests {
    extern crate test;
    use self::test::Bencher;
    use boxed::Box;
    use alloc::{Global, Alloc, Layout};

    #[test]
    fn allocate_zeroed() {
        unsafe {
            let layout = Layout::from_size_align(1024, 1).unwrap();
            let ptr = Global.alloc_zeroed(layout.clone())
                .unwrap_or_else(|_| Global.oom());

            let end = ptr.offset(layout.size() as isize);
            let mut i = ptr;
            while i < end {
                assert_eq!(*i, 0);
                i = i.offset(1);
            }
            Global.dealloc(ptr, layout);
        }
    }

    #[bench]
    fn alloc_owned_small(b: &mut Bencher) {
        b.iter(|| {
            let _: Box<_> = box 10;
        })
    }
}
