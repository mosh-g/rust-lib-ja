// Copyright 2014 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.
//
// ignore-lexer-test FIXME #15679

//! Regular expressions implemented in Rust
//!
//! For official documentation, see the rust-lang/regex crate
#![crate_name = "regex"]
#![crate_type = "rlib"]
#![crate_type = "dylib"]
#![unstable(feature = "rustc_private",
            reason = "use the crates.io `regex` library instead")]
#![feature(staged_api)]
#![staged_api]
#![doc(html_logo_url = "http://www.rust-lang.org/logos/rust-logo-128x128-blk-v2.png",
       html_favicon_url = "http://www.rust-lang.org/favicon.ico",
       html_root_url = "http://doc.rust-lang.org/nightly/",
       html_playground_url = "http://play.rust-lang.org/")]

#![allow(unknown_features)]
#![feature(slicing_syntax)]
#![feature(box_syntax)]
#![allow(unknown_features)] #![feature(int_uint)]
#![deny(missing_docs)]
#![feature(collections)]
#![feature(core)]
#![feature(unicode)]

#[cfg(test)]
extern crate "test" as stdtest;
#[cfg(test)]
extern crate rand;

// During tests, this links with the `regex` crate so that the `regex!` macro
// can be tested.
#[cfg(test)]
extern crate regex;

// Unicode tables for character classes are defined in libunicode
extern crate unicode;

pub use parse::Error;
pub use re::{Regex, Captures, SubCaptures, SubCapturesPos};
pub use re::{FindCaptures, FindMatches};
pub use re::{Replacer, NoExpand, RegexSplits, RegexSplitsN};
pub use re::{quote, is_match};

mod compile;
mod parse;
mod re;
mod vm;

#[cfg(test)]
mod test;

/// The `native` module exists to support the `regex!` macro. Do not use.
#[doc(hidden)]
pub mod native {
    // Exporting this stuff is bad form, but it's necessary for two reasons.
    // Firstly, the `regex!` syntax extension is in a different crate and
    // requires access to the representation of a regex (particularly the
    // instruction set) in order to compile to native Rust. This could be
    // mitigated if `regex!` was defined in the same crate, but this has
    // undesirable consequences (such as requiring a dependency on
    // `libsyntax`).
    //
    // Secondly, the code generated by `regex!` must *also* be able
    // to access various functions in this crate to reduce code duplication
    // and to provide a value with precisely the same `Regex` type in this
    // crate. This, AFAIK, is impossible to mitigate.
    //
    // On the bright side, `rustdoc` lets us hide this from the public API
    // documentation.
    pub use compile::{
        Program,
        OneChar, CharClass, Any, Save, Jump, Split,
        Match, EmptyBegin, EmptyEnd, EmptyWordBoundary,
    };
    pub use parse::{
        FLAG_EMPTY, FLAG_NOCASE, FLAG_MULTI, FLAG_DOTNL,
        FLAG_SWAP_GREED, FLAG_NEGATED,
    };
    pub use re::{Dynamic, ExDynamic, Native, ExNative};
    pub use vm::{
        MatchKind, Exists, Location, Submatches,
        StepState, StepMatchEarlyReturn, StepMatch, StepContinue,
        CharReader, find_prefix,
    };
}
