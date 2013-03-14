// Copyright 2013 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.


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

mod sched;
mod rtio;
pub mod uvll;
mod uvio;
mod uv;
// FIXME #5248: The import in `sched` doesn't resolve unless this is pub!
pub mod thread_local_storage;
mod work_queue;
mod stack;
mod context;
mod thread;
pub mod env;
