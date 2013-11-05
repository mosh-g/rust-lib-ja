// Copyright 2013 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

/*!

Bindings to libuv, along with the default implementation of `std::rt::rtio`.

UV types consist of the event loop (Loop), Watchers, Requests and
Callbacks.

Watchers and Requests encapsulate pointers to uv *handles*, which have
subtyping relationships with each other.  This subtyping is reflected
in the bindings with explicit or implicit coercions. For example, an
upcast from TcpWatcher to StreamWatcher is done with
`tcp_watcher.as_stream()`. In other cases a callback on a specific
type of watcher will be passed a watcher of a supertype.

Currently all use of Request types (connect/write requests) are
encapsulated in the bindings and don't need to be dealt with by the
caller.

# Safety note

Due to the complex lifecycle of uv handles, as well as compiler bugs,
this module is not memory safe and requires explicit memory management,
via `close` and `delete` methods.

*/

#[link(name = "rustuv",
       package_id = "rustuv",
       vers = "0.9-pre",
       uuid = "f3719011-0459-9b86-b11c-29265c0d0864",
       url = "https://github.com/mozilla/rust/tree/master/src/librustuv")];

#[license = "MIT/ASL2"];
#[crate_type = "lib"];

#[feature(macro_rules, globs)];

use std::cast;
use std::str::raw::from_c_str;
use std::vec;
use std::ptr;
use std::str;
use std::libc::{c_void, c_int, size_t, malloc, free};
use std::cast::transmute;
use std::ptr::null;
use std::unstable::finally::Finally;
use std::rt::io::net::ip::SocketAddr;

use std::rt::io::IoError;

//#[cfg(test)] use unstable::run_in_bare_thread;

pub use self::file::{FsRequest};
pub use self::net::{StreamWatcher, TcpWatcher, UdpWatcher};
pub use self::idle::IdleWatcher;
pub use self::timer::TimerWatcher;
pub use self::async::AsyncWatcher;
pub use self::process::Process;
pub use self::pipe::PipeWatcher;
pub use self::signal::SignalWatcher;
pub use self::tty::TtyWatcher;

mod macros;

/// The implementation of `rtio` for libuv
pub mod uvio;

/// C bindings to libuv
pub mod uvll;

pub mod file;
pub mod net;
pub mod idle;
pub mod timer;
pub mod async;
pub mod addrinfo;
pub mod process;
pub mod pipe;
pub mod tty;
pub mod signal;
pub mod stream;

/// XXX: Loop(*handle) is buggy with destructors. Normal structs
/// with dtors may not be destructured, but tuple structs can,
/// but the results are not correct.
pub struct Loop {
    priv handle: *uvll::uv_loop_t
}

pub struct Handle(*uvll::uv_handle_t);

impl Watcher for Handle {}
impl NativeHandle<*uvll::uv_handle_t> for Handle {
    fn from_native_handle(h: *uvll::uv_handle_t) -> Handle { Handle(h) }
    fn native_handle(&self) -> *uvll::uv_handle_t { **self }
}

/// The trait implemented by uv 'watchers' (handles). Watchers are
/// non-owning wrappers around the uv handles and are not completely
/// safe - there may be multiple instances for a single underlying
/// handle.  Watchers are generally created, then `start`ed, `stop`ed
/// and `close`ed, but due to their complex life cycle may not be
/// entirely memory safe if used in unanticipated patterns.
pub trait Watcher { }

pub trait Request { }

/// A type that wraps a native handle
pub trait NativeHandle<T> {
    fn from_native_handle(T) -> Self;
    fn native_handle(&self) -> T;
}

/// A type that wraps a uv handle
pub trait UvHandle<T> {
    fn uv_handle(&self) -> *T;

    // FIXME(#8888) dummy self
    fn alloc(_: Option<Self>, ty: uvll::uv_handle_type) -> *T {
        unsafe {
            let handle = uvll::malloc_handle(ty);
            assert!(!handle.is_null());
            handle as *T
        }
    }

    unsafe fn from_uv_handle<'a>(h: &'a *T) -> &'a mut Self {
        cast::transmute(uvll::get_data_for_uv_handle(*h))
    }

    fn install(~self) -> ~Self {
        unsafe {
            let myptr = cast::transmute::<&~Self, &*u8>(&self);
            uvll::set_data_for_uv_handle(self.uv_handle(), *myptr);
        }
        self
    }

    fn close_async_(&mut self) {
        // we used malloc to allocate all handles, so we must always have at
        // least a callback to free all the handles we allocated.
        extern fn close_cb(handle: *uvll::uv_handle_t) {
            unsafe { uvll::free_handle(handle) }
        }

        unsafe {
            uvll::set_data_for_uv_handle(self.uv_handle(), null::<()>());
            uvll::uv_close(self.uv_handle() as *uvll::uv_handle_t, close_cb)
        }
    }
}

pub trait UvRequest<T> {
    fn uv_request(&self) -> *T;

    // FIXME(#8888) dummy self
    fn alloc(_: Option<Self>, ty: uvll::uv_req_type) -> *T {
        unsafe {
            let handle = uvll::malloc_req(ty);
            assert!(!handle.is_null());
            handle as *T
        }
    }

    unsafe fn from_uv_request<'a>(h: &'a *T) -> &'a mut Self {
        cast::transmute(uvll::get_data_for_req(*h))
    }

    fn install(~self) -> ~Self {
        unsafe {
            let myptr = cast::transmute::<&~Self, &*u8>(&self);
            uvll::set_data_for_req(self.uv_request(), *myptr);
        }
        self
    }

    fn delete(&mut self) {
        unsafe { uvll::free_req(self.uv_request() as *c_void) }
    }
}

impl Loop {
    pub fn new() -> Loop {
        let handle = unsafe { uvll::loop_new() };
        assert!(handle.is_not_null());
        NativeHandle::from_native_handle(handle)
    }

    pub fn run(&mut self) {
        unsafe { uvll::uv_run(self.native_handle(), uvll::RUN_DEFAULT) };
    }

    pub fn close(&mut self) {
        unsafe { uvll::uv_loop_delete(self.native_handle()) };
    }
}

impl NativeHandle<*uvll::uv_loop_t> for Loop {
    fn from_native_handle(handle: *uvll::uv_loop_t) -> Loop {
        Loop { handle: handle }
    }
    fn native_handle(&self) -> *uvll::uv_loop_t {
        self.handle
    }
}

// XXX: The uv alloc callback also has a *uv_handle_t arg
pub type AllocCallback = ~fn(uint) -> Buf;
pub type ReadCallback = ~fn(StreamWatcher, int, Buf, Option<UvError>);
pub type NullCallback = ~fn();
pub type ConnectionCallback = ~fn(StreamWatcher, Option<UvError>);
pub type FsCallback = ~fn(&mut FsRequest, Option<UvError>);
pub type UdpReceiveCallback = ~fn(UdpWatcher, int, Buf, SocketAddr, uint, Option<UvError>);
pub type UdpSendCallback = ~fn(UdpWatcher, Option<UvError>);


/// Callbacks used by StreamWatchers, set as custom data on the foreign handle.
/// XXX: Would be better not to have all watchers allocate room for all callback types.
struct WatcherData {
    read_cb: Option<ReadCallback>,
    write_cb: Option<ConnectionCallback>,
    connect_cb: Option<ConnectionCallback>,
    close_cb: Option<NullCallback>,
    alloc_cb: Option<AllocCallback>,
    udp_recv_cb: Option<UdpReceiveCallback>,
    udp_send_cb: Option<UdpSendCallback>,
}

pub trait WatcherInterop {
    fn event_loop(&self) -> Loop;
    fn install_watcher_data(&mut self);
    fn get_watcher_data<'r>(&'r mut self) -> &'r mut WatcherData;
    fn drop_watcher_data(&mut self);
    fn close(self, cb: NullCallback);
    fn close_async(self);
}

impl<H, W: Watcher + NativeHandle<*H>> WatcherInterop for W {
    /// Get the uv event loop from a Watcher
    fn event_loop(&self) -> Loop {
        unsafe {
            let handle = self.native_handle();
            let loop_ = uvll::get_loop_for_uv_handle(handle);
            NativeHandle::from_native_handle(loop_)
        }
    }

    fn install_watcher_data(&mut self) {
        unsafe {
            let data = ~WatcherData {
                read_cb: None,
                write_cb: None,
                connect_cb: None,
                close_cb: None,
                alloc_cb: None,
                async_cb: None,
                udp_recv_cb: None,
                udp_send_cb: None,
            };
            let data = transmute::<~WatcherData, *c_void>(data);
            uvll::set_data_for_uv_handle(self.native_handle(), data);
        }
    }

    fn get_watcher_data<'r>(&'r mut self) -> &'r mut WatcherData {
        unsafe {
            let data = uvll::get_data_for_uv_handle(self.native_handle());
            let data = transmute::<&*c_void, &mut ~WatcherData>(&data);
            return &mut **data;
        }
    }

    fn drop_watcher_data(&mut self) {
        unsafe {
            let data = uvll::get_data_for_uv_handle(self.native_handle());
            let _data = transmute::<*c_void, ~WatcherData>(data);
            uvll::set_data_for_uv_handle(self.native_handle(), null::<()>());
        }
    }

    fn close(mut self, cb: NullCallback) {
        {
            let data = self.get_watcher_data();
            assert!(data.close_cb.is_none());
            data.close_cb = Some(cb);
        }

        unsafe {
            uvll::uv_close(self.native_handle() as *uvll::uv_handle_t, close_cb);
        }

        extern fn close_cb(handle: *uvll::uv_handle_t) {
            let mut h: Handle = NativeHandle::from_native_handle(handle);
            h.get_watcher_data().close_cb.take_unwrap()();
            h.drop_watcher_data();
            unsafe { uvll::free_handle(handle as *c_void) }
        }
    }

    fn close_async(self) {
        unsafe {
            uvll::uv_close(self.native_handle() as *uvll::uv_handle_t, close_cb);
        }

        extern fn close_cb(handle: *uvll::uv_handle_t) {
            let mut h: Handle = NativeHandle::from_native_handle(handle);
            h.drop_watcher_data();
            unsafe { uvll::free_handle(handle as *c_void) }
        }
    }
}

// XXX: Need to define the error constants like EOF so they can be
// compared to the UvError type

pub struct UvError(c_int);

impl UvError {
    pub fn name(&self) -> ~str {
        unsafe {
            let inner = match self { &UvError(a) => a };
            let name_str = uvll::uv_err_name(inner);
            assert!(name_str.is_not_null());
            from_c_str(name_str)
        }
    }

    pub fn desc(&self) -> ~str {
        unsafe {
            let inner = match self { &UvError(a) => a };
            let desc_str = uvll::uv_strerror(inner);
            assert!(desc_str.is_not_null());
            from_c_str(desc_str)
        }
    }

    pub fn is_eof(&self) -> bool {
        **self == uvll::EOF
    }
}

impl ToStr for UvError {
    fn to_str(&self) -> ~str {
        format!("{}: {}", self.name(), self.desc())
    }
}

#[test]
fn error_smoke_test() {
    let err: UvError = UvError(uvll::EOF);
    assert_eq!(err.to_str(), ~"EOF: end of file");
}

pub fn uv_error_to_io_error(uverr: UvError) -> IoError {
    unsafe {
        // Importing error constants
        use uvll::*;
        use std::rt::io::*;

        // uv error descriptions are static
        let c_desc = uvll::uv_strerror(*uverr);
        let desc = str::raw::c_str_to_static_slice(c_desc);

        let kind = match *uverr {
            UNKNOWN => OtherIoError,
            OK => OtherIoError,
            EOF => EndOfFile,
            EACCES => PermissionDenied,
            ECONNREFUSED => ConnectionRefused,
            ECONNRESET => ConnectionReset,
            ENOTCONN => NotConnected,
            EPIPE => BrokenPipe,
            ECONNABORTED => ConnectionAborted,
            err => {
                uvdebug!("uverr.code {}", err as int);
                // XXX: Need to map remaining uv error types
                OtherIoError
            }
        };

        IoError {
            kind: kind,
            desc: desc,
            detail: None
        }
    }
}

/// Given a uv error code, convert a callback status to a UvError
pub fn status_to_maybe_uv_error(status: c_int) -> Option<UvError> {
    if status >= 0 {
        None
    } else {
        Some(UvError(status))
    }
}

pub fn status_to_io_result(status: c_int) -> Result<(), IoError> {
    if status >= 0 {Ok(())} else {Err(uv_error_to_io_error(UvError(status)))}
}

/// The uv buffer type
pub type Buf = uvll::uv_buf_t;

pub fn empty_buf() -> Buf {
    uvll::uv_buf_t {
        base: null(),
        len: 0,
    }
}

/// Borrow a slice to a Buf
pub fn slice_to_uv_buf(v: &[u8]) -> Buf {
    let data = vec::raw::to_ptr(v);
    uvll::uv_buf_t { base: data, len: v.len() as uvll::uv_buf_len_t }
}

// XXX: Do these conversions without copying

/// Transmute an owned vector to a Buf
pub fn vec_to_uv_buf(v: ~[u8]) -> Buf {
    #[fixed_stack_segment]; #[inline(never)];

    unsafe {
        let data = malloc(v.len() as size_t) as *u8;
        assert!(data.is_not_null());
        do v.as_imm_buf |b, l| {
            let data = data as *mut u8;
            ptr::copy_memory(data, b, l)
        }
        uvll::uv_buf_t { base: data, len: v.len() as uvll::uv_buf_len_t }
    }
}

/// Transmute a Buf that was once a ~[u8] back to ~[u8]
pub fn vec_from_uv_buf(buf: Buf) -> Option<~[u8]> {
    #[fixed_stack_segment]; #[inline(never)];

    if !(buf.len == 0 && buf.base.is_null()) {
        let v = unsafe { vec::from_buf(buf.base, buf.len as uint) };
        unsafe { free(buf.base as *c_void) };
        return Some(v);
    } else {
        // No buffer
        uvdebug!("No buffer!");
        return None;
    }
}
/*
#[test]
fn test_slice_to_uv_buf() {
    let slice = [0, .. 20];
    let buf = slice_to_uv_buf(slice);

    assert!(buf.len == 20);

    unsafe {
        let base = transmute::<*u8, *mut u8>(buf.base);
        (*base) = 1;
        (*ptr::mut_offset(base, 1)) = 2;
    }

    assert!(slice[0] == 1);
    assert!(slice[1] == 2);
}


#[test]
fn loop_smoke_test() {
    do run_in_bare_thread {
        let mut loop_ = Loop::new();
        loop_.run();
        loop_.close();
    }
}
*/
