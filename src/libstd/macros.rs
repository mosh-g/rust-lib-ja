// Copyright 2012 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

#[macro_escape];

// Some basic logging
macro_rules! rtdebug_ (
    ($( $arg:expr),+) => ( {
        dumb_println(fmt!( $($arg),+ ));

        fn dumb_println(s: &str) {
            use io::WriterUtil;
            let dbg = ::libc::STDERR_FILENO as ::io::fd_t;
            dbg.write_str(s);
            dbg.write_str("\n");
        }

    } )
)

// An alternate version with no output, for turning off logging
macro_rules! rtdebug (
    ($( $arg:expr),+) => ( $(let _ = $arg)*; )
)

macro_rules! rtassert (
    ( $arg:expr ) => ( {
        if !$arg {
            abort!("assertion failed: %s", stringify!($arg));
        }
    } )
)


// The do_abort function was originally inside the abort macro, but
// this was ICEing the compiler so it has been moved outside. Now this
// seems to work?
pub fn do_abort() -> ! {
    unsafe { ::libc::abort(); }
}

macro_rules! abort(
    ($( $msg:expr),+) => ( {
        rtdebug!($($msg),+);
        ::macros::do_abort();
    } )
)

