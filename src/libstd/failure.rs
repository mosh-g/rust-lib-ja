// Copyright 2014 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

#![experimental]

use alloc::boxed::Box;
use any::{Any, AnyRefExt};
use cell::RefCell;
use fmt;
use io::{Writer, IoResult};
use kinds::Send;
use option::Option;
use option::Option::{Some, None};
use result::Result::Ok;
use rt::backtrace;
use rt::util::{Stderr, Stdio};
use str::Str;
use string::String;
use thread::Thread;
use sys_common::thread_info;

// Defined in this module instead of io::stdio so that the unwinding
thread_local! {
    pub static LOCAL_STDERR: RefCell<Option<Box<Writer + Send>>> = {
        RefCell::new(None)
    }
}

impl Writer for Stdio {
    fn write(&mut self, bytes: &[u8]) -> IoResult<()> {
        fn fmt_write<F: fmt::FormatWriter>(f: &mut F, bytes: &[u8]) {
            let _ = f.write(bytes);
        }
        fmt_write(self, bytes);
        Ok(())
    }
}

pub fn on_fail(obj: &(Any+Send), file: &'static str, line: uint) {
    let msg = match obj.downcast_ref::<&'static str>() {
        Some(s) => *s,
        None => match obj.downcast_ref::<String>() {
            Some(s) => s.as_slice(),
            None => "Box<Any>",
        }
    };
    let mut err = Stderr;
    let thread = Thread::current();
    let name = thread.name().unwrap_or("<unnamed>");
    let prev = LOCAL_STDERR.with(|s| s.borrow_mut().take());
    match prev {
        Some(mut stderr) => {
            // FIXME: what to do when the thread printing panics?
            let _ = writeln!(stderr,
                             "thread '{}' panicked at '{}', {}:{}\n",
                             name, msg, file, line);
            if backtrace::log_enabled() {
                let _ = backtrace::write(&mut *stderr);
            }
            let mut s = Some(stderr);
            LOCAL_STDERR.with(|slot| {
                *slot.borrow_mut() = s.take();
            });
        }
        None => {
            let _ = writeln!(&mut err, "thread '{}' panicked at '{}', {}:{}",
                             name, msg, file, line);
            if backtrace::log_enabled() {
                let _ = backtrace::write(&mut err);
            }
        }
    }

    // If this is a double panic, make sure that we printed a backtrace
    // for this panic.
    if thread_info::panicking() && !backtrace::log_enabled() {
        let _ = backtrace::write(&mut err);
    }
}
