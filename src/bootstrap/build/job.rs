// Copyright 2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Job management on Windows for bootstrapping
//!
//! Most of the time when you're running a build system (e.g. make) you expect
//! Ctrl-C or abnormal termination to actually terminate the entire tree of
//! process in play, not just the one at the top. This currently works "by
//! default" on Unix platforms because Ctrl-C actually sends a signal to the
//! *process group* rather than the parent process, so everything will get torn
//! down. On Windows, however, this does not happen and Ctrl-C just kills the
//! parent process.
//!
//! To achieve the same semantics on Windows we use Job Objects to ensure that
//! all processes die at the same time. Job objects have a mode of operation
//! where when all handles to the object are closed it causes all child
//! processes associated with the object to be terminated immediately.
//! Conveniently whenever a process in the job object spawns a new process the
//! child will be associated with the job object as well. This means if we add
//! ourselves to the job object we create then everything will get torn down!
//!
//! Unfortunately most of the time the build system is actually called from a
//! python wrapper (which manages things like building the build system) so this
//! all doesn't quite cut it so far. To go the last mile we duplicate the job
//! object handle into our parent process (a python process probably) and then
//! close our own handle. This means that the only handle to the job object
//! resides in the parent python process, so when python dies the whole build
//! system dies (as one would probably expect!).
//!
//! Note that this module has a #[cfg(windows)] above it as none of this logic
//! is required on Unix.

extern crate kernel32;
extern crate winapi;

use std::env;
use std::io;
use std::mem;

use self::winapi::*;
use self::kernel32::*;

pub unsafe fn setup() {
    // Create a new job object for us to use
    let job = CreateJobObjectW(0 as *mut _, 0 as *const _);
    assert!(job != 0 as *mut _, "{}", io::Error::last_os_error());

    // Indicate that when all handles to the job object are gone that all
    // process in the object should be killed. Note that this includes our
    // entire process tree by default because we've added ourselves and and our
    // children will reside in the job by default.
    let mut info = mem::zeroed::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>();
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let r = SetInformationJobObject(job,
                                    JobObjectExtendedLimitInformation,
                                    &mut info as *mut _ as LPVOID,
                                    mem::size_of_val(&info) as DWORD);
    assert!(r != 0, "{}", io::Error::last_os_error());

    // Assign our process to this job object. Note that if this fails, one very
    // likely reason is that we are ourselves already in a job object! This can
    // happen on the build bots that we've got for Windows, or if just anyone
    // else is instrumenting the build. In this case we just bail out
    // immediately and assume that they take care of it.
    //
    // Also note that nested jobs (why this might fail) are supported in recent
    // versions of Windows, but the version of Windows that our bots are running
    // at least don't support nested job objects.
    let r = AssignProcessToJobObject(job, GetCurrentProcess());
    if r == 0 {
        CloseHandle(job);
        return
    }

    // If we've got a parent process (e.g. the python script that called us)
    // then move ownership of this job object up to them. That way if the python
    // script is killed (e.g. via ctrl-c) then we'll all be torn down.
    //
    // If we don't have a parent (e.g. this was run directly) then we
    // intentionally leak the job object handle. When our process exits
    // (normally or abnormally) it will close the handle implicitly, causing all
    // processes in the job to be cleaned up.
    let pid = match env::var("BOOTSTRAP_PARENT_ID") {
        Ok(s) => s,
        Err(..) => return,
    };

    let parent = OpenProcess(PROCESS_DUP_HANDLE, FALSE, pid.parse().unwrap());
    assert!(parent != 0 as *mut _, "{}", io::Error::last_os_error());
    let mut parent_handle = 0 as *mut _;
    let r = DuplicateHandle(GetCurrentProcess(), job,
                            parent, &mut parent_handle,
                            0, FALSE, DUPLICATE_SAME_ACCESS);

    // If this failed, well at least we tried! An example of DuplicateHandle
    // failing in the past has been when the wrong python2 package spawed this
    // build system (e.g. the `python2` package in MSYS instead of
    // `mingw-w64-x86_64-python2`. Not sure why it failed, but the "failure
    // mode" here is that we only clean everything up when the build system
    // dies, not when the python parent does, so not too bad.
    if r != 0 {
        CloseHandle(job);
    }
}
