// Copyright 2014 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! # The Rust Core Library
//!
//! The Rust Core Library is the dependency-free foundation of [The
//! Rust Standard Library](../std/index.html). It is the portable glue
//! between the language and its libraries, defining the intrinsic and
//! primitive building blocks of all Rust code. It links to no
//! upstream libraries, no system libraries, and no libc.
//!
//! The core library is *minimal*: it isn't even aware of heap allocation,
//! nor does it provide concurrency or I/O. These things require
//! platform integration, and this library is platform-agnostic.
//!
//! *It is not recommended to use the core library*. The stable
//! functionality of libcore is reexported from the
//! [standard library](../std/index.html). The composition of this library is
//! subject to change over time; only the interface exposed through libstd is
//! intended to be stable.
//!
//! # How to use the core library
//!
// FIXME: Fill me in with more detail when the interface settles
//! This library is built on the assumption of a few existing symbols:
//!
//! * `memcpy`, `memcmp`, `memset` - These are core memory routines which are
//!   often generated by LLVM. Additionally, this library can make explicit
//!   calls to these functions. Their signatures are the same as found in C.
//!   These functions are often provided by the system libc, but can also be
//!   provided by the [rlibc crate](https://crates.io/crates/rlibc).
//!
//! * `rust_begin_unwind` - This function takes three arguments, a
//!   `fmt::Arguments`, a `&str`, and a `usize`. These three arguments dictate
//!   the panic message, the file at which panic was invoked, and the line.
//!   It is up to consumers of this core library to define this panic
//!   function; it is only required to never return.

// Since libcore defines many fundamental lang items, all tests live in a
// separate crate, libcoretest, to avoid bizarre issues.

// Do not remove on snapshot creation. Needed for bootstrap. (Issue #22364)
#![cfg_attr(stage0, feature(custom_attribute))]
#![crate_name = "core"]
#![unstable(feature = "core")]
#![staged_api]
#![crate_type = "rlib"]
#![doc(html_logo_url = "http://www.rust-lang.org/logos/rust-logo-128x128-blk-v2.png",
       html_favicon_url = "http://www.rust-lang.org/favicon.ico",
       html_root_url = "http://doc.rust-lang.org/nightly/",
       html_playground_url = "http://play.rust-lang.org/")]
#![doc(test(no_crate_inject))]

#![feature(no_std)]
#![no_std]
#![allow(raw_pointer_derive)]
#![deny(missing_docs)]

#![feature(int_uint)]
#![feature(intrinsics, lang_items)]
#![feature(on_unimplemented)]
#![feature(simd, unsafe_destructor)]
#![feature(staged_api)]
#![feature(unboxed_closures)]
#![feature(rustc_attrs)]
#![feature(optin_builtin_traits)]
#![feature(concat_idents)]

#[macro_use]
mod macros;

#[macro_use]
mod cmp_macros;

#[path = "num/float_macros.rs"]
#[macro_use]
mod float_macros;

#[path = "num/int_macros.rs"]
#[macro_use]
mod int_macros;

#[path = "num/uint_macros.rs"]
#[macro_use]
mod uint_macros;

#[path = "num/isize.rs"]  pub mod isize;
#[path = "num/i8.rs"]   pub mod i8;
#[path = "num/i16.rs"]  pub mod i16;
#[path = "num/i32.rs"]  pub mod i32;
#[path = "num/i64.rs"]  pub mod i64;

#[path = "num/usize.rs"] pub mod usize;
#[path = "num/u8.rs"]   pub mod u8;
#[path = "num/u16.rs"]  pub mod u16;
#[path = "num/u32.rs"]  pub mod u32;
#[path = "num/u64.rs"]  pub mod u64;

#[path = "num/f32.rs"]   pub mod f32;
#[path = "num/f64.rs"]   pub mod f64;

pub mod num;

/* The libcore prelude, not as all-encompassing as the libstd prelude */

pub mod prelude;

/* Core modules for ownership management */

pub mod intrinsics;
pub mod mem;
pub mod nonzero;
pub mod ptr;

/* Core language traits */

pub mod marker;
pub mod ops;
pub mod cmp;
pub mod clone;
pub mod default;

/* Core types and methods on primitives */

pub mod any;
pub mod array;
pub mod atomic;
pub mod cell;
pub mod char;
pub mod panicking;
pub mod finally;
pub mod iter;
pub mod option;
pub mod raw;
pub mod result;
pub mod simd;
pub mod slice;
pub mod str;
pub mod hash;
pub mod fmt;
pub mod error;

#[doc(primitive = "bool")]
mod bool {
}

// note: does not need to be public
mod tuple;

#[doc(hidden)]
mod core {
    pub use panicking;
    pub use fmt;
    pub use clone;
    pub use cmp;
    pub use hash;
    pub use marker;
    pub use option;
    pub use iter;
}

#[doc(hidden)]
mod std {
    // range syntax
    pub use ops;
}
