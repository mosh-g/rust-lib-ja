// Copyright 2012 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

/*!

More runtime type reflection

*/

#[allow(missing_doc)];

use cast::transmute;
use char;
use container::Container;
use rt::io;
use iterator::Iterator;
use libc::c_void;
use option::{Some, None};
use ptr;
use reflect;
use reflect::{MovePtr, align};
use str::StrSlice;
use to_str::ToStr;
use vec::OwnedVector;
use unstable::intrinsics::{Opaque, TyDesc, TyVisitor, get_tydesc, visit_tydesc};
use unstable::raw;

/// Representations

trait Repr {
    fn write_repr(&self, writer: &mut io::Writer);
}

impl Repr for () {
    fn write_repr(&self, writer: &mut io::Writer) {
        writer.write("()".as_bytes());
    }
}

impl Repr for bool {
    fn write_repr(&self, writer: &mut io::Writer) {
        let s = if *self { "true" } else { "false" };
        writer.write(s.as_bytes())
    }
}

impl Repr for int {
    fn write_repr(&self, writer: &mut io::Writer) {
        do ::int::to_str_bytes(*self, 10u) |bits| {
            writer.write(bits);
        }
    }
}

macro_rules! int_repr(($ty:ident, $suffix:expr) => (impl Repr for $ty {
    fn write_repr(&self, writer: &mut io::Writer) {
        do ::$ty::to_str_bytes(*self, 10u) |bits| {
            writer.write(bits);
            writer.write(bytes!($suffix));
        }
    }
}))

int_repr!(i8, "i8")
int_repr!(i16, "i16")
int_repr!(i32, "i32")
int_repr!(i64, "i64")
int_repr!(uint, "u")
int_repr!(u8, "u8")
int_repr!(u16, "u16")
int_repr!(u32, "u32")
int_repr!(u64, "u64")

impl Repr for float {
    fn write_repr(&self, writer: &mut io::Writer) {
        let s = self.to_str();
        writer.write(s.as_bytes());
    }
}

macro_rules! num_repr(($ty:ident, $suffix:expr) => (impl Repr for $ty {
    fn write_repr(&self, writer: &mut io::Writer) {
        let s = self.to_str();
        writer.write(s.as_bytes());
        writer.write(bytes!($suffix));
    }
}))

num_repr!(f32, "f32")
num_repr!(f64, "f64")

// New implementation using reflect::MovePtr

enum VariantState {
    SearchingFor(int),
    Matched,
    AlreadyFound
}

pub struct ReprVisitor<'self> {
    ptr: *c_void,
    ptr_stk: ~[*c_void],
    var_stk: ~[VariantState],
    writer: &'self mut io::Writer
}

pub fn ReprVisitor<'a>(ptr: *c_void,
                       writer: &'a mut io::Writer) -> ReprVisitor<'a> {
    ReprVisitor {
        ptr: ptr,
        ptr_stk: ~[],
        var_stk: ~[],
        writer: writer,
    }
}

impl<'self> MovePtr for ReprVisitor<'self> {
    #[inline]
    fn move_ptr(&mut self, adjustment: &fn(*c_void) -> *c_void) {
        self.ptr = adjustment(self.ptr);
    }
    fn push_ptr(&mut self) {
        self.ptr_stk.push(self.ptr);
    }
    fn pop_ptr(&mut self) {
        self.ptr = self.ptr_stk.pop();
    }
}

impl<'self> ReprVisitor<'self> {
    // Various helpers for the TyVisitor impl

    #[inline]
    pub fn get<T>(&mut self, f: &fn(&mut ReprVisitor, &T)) -> bool {
        unsafe {
            f(self, transmute::<*c_void,&T>(self.ptr));
        }
        true
    }

    #[inline]
    pub fn visit_inner(&mut self, inner: *TyDesc) -> bool {
        self.visit_ptr_inner(self.ptr, inner)
    }

    #[inline]
    pub fn visit_ptr_inner(&mut self, ptr: *c_void, inner: *TyDesc) -> bool {
        unsafe {
            // This should call the constructor up above, but due to limiting
            // issues we have to recreate it here.
            let u = ReprVisitor {
                ptr: ptr,
                ptr_stk: ~[],
                var_stk: ~[],
                writer: ::cast::transmute_copy(&self.writer),
            };
            let mut v = reflect::MovePtrAdaptor(u);
            // Obviously this should not be a thing, but blame #8401 for now
            visit_tydesc(inner, &mut v as &mut TyVisitor);
            true
        }
    }

    #[inline]
    pub fn write<T:Repr>(&mut self) -> bool {
        do self.get |this, v:&T| {
            v.write_repr(unsafe { ::cast::transmute_copy(&this.writer) });
        }
    }

    pub fn write_escaped_slice(&mut self, slice: &str) {
        self.writer.write(['"' as u8]);
        for ch in slice.iter() {
            self.write_escaped_char(ch, true);
        }
        self.writer.write(['"' as u8]);
    }

    pub fn write_mut_qualifier(&mut self, mtbl: uint) {
        if mtbl == 0 {
            self.writer.write("mut ".as_bytes());
        } else if mtbl == 1 {
            // skip, this is ast::m_imm
        } else {
            fail!("invalid mutability value");
        }
    }

    pub fn write_vec_range(&mut self,
                           _mtbl: uint,
                           ptr: *(),
                           len: uint,
                           inner: *TyDesc)
                           -> bool {
        let mut p = ptr as *u8;
        let (sz, al) = unsafe { ((*inner).size, (*inner).align) };
        self.writer.write(['[' as u8]);
        let mut first = true;
        let mut left = len;
        // unit structs have 0 size, and don't loop forever.
        let dec = if sz == 0 {1} else {sz};
        while left > 0 {
            if first {
                first = false;
            } else {
                self.writer.write(", ".as_bytes());
            }
            self.visit_ptr_inner(p as *c_void, inner);
            p = align(unsafe { ptr::offset(p, sz as int) as uint }, al) as *u8;
            left -= dec;
        }
        self.writer.write([']' as u8]);
        true
    }

    pub fn write_unboxed_vec_repr(&mut self,
                                  mtbl: uint,
                                  v: &raw::Vec<()>,
                                  inner: *TyDesc)
                                  -> bool {
        self.write_vec_range(mtbl, ptr::to_unsafe_ptr(&v.data),
                             v.fill, inner)
    }

    fn write_escaped_char(&mut self, ch: char, is_str: bool) {
        match ch {
            '\t' => self.writer.write("\\t".as_bytes()),
            '\r' => self.writer.write("\\r".as_bytes()),
            '\n' => self.writer.write("\\n".as_bytes()),
            '\\' => self.writer.write("\\\\".as_bytes()),
            '\'' => {
                if is_str {
                    self.writer.write("'".as_bytes())
                } else {
                    self.writer.write("\\'".as_bytes())
                }
            }
            '"' => {
                if is_str {
                    self.writer.write("\\\"".as_bytes())
                } else {
                    self.writer.write("\"".as_bytes())
                }
            }
            '\x20'..'\x7e' => self.writer.write([ch as u8]),
            _ => {
                do char::escape_unicode(ch) |c| {
                    self.writer.write([c as u8]);
                }
            }
        }
    }
}

impl<'self> TyVisitor for ReprVisitor<'self> {
    fn visit_bot(&mut self) -> bool {
        self.writer.write("!".as_bytes());
        true
    }
    fn visit_nil(&mut self) -> bool { self.write::<()>() }
    fn visit_bool(&mut self) -> bool { self.write::<bool>() }
    fn visit_int(&mut self) -> bool { self.write::<int>() }
    fn visit_i8(&mut self) -> bool { self.write::<i8>() }
    fn visit_i16(&mut self) -> bool { self.write::<i16>() }
    fn visit_i32(&mut self) -> bool { self.write::<i32>()  }
    fn visit_i64(&mut self) -> bool { self.write::<i64>() }

    fn visit_uint(&mut self) -> bool { self.write::<uint>() }
    fn visit_u8(&mut self) -> bool { self.write::<u8>() }
    fn visit_u16(&mut self) -> bool { self.write::<u16>() }
    fn visit_u32(&mut self) -> bool { self.write::<u32>() }
    fn visit_u64(&mut self) -> bool { self.write::<u64>() }

    fn visit_float(&mut self) -> bool { self.write::<float>() }
    fn visit_f32(&mut self) -> bool { self.write::<f32>() }
    fn visit_f64(&mut self) -> bool { self.write::<f64>() }

    fn visit_char(&mut self) -> bool {
        do self.get::<char> |this, &ch| {
            this.writer.write(['\'' as u8]);
            this.write_escaped_char(ch, false);
            this.writer.write(['\'' as u8]);
        }
    }

    fn visit_estr_box(&mut self) -> bool {
        do self.get::<@str> |this, s| {
            this.writer.write(['@' as u8]);
            this.write_escaped_slice(*s);
        }
    }

    fn visit_estr_uniq(&mut self) -> bool {
        do self.get::<~str> |this, s| {
            this.writer.write(['~' as u8]);
            this.write_escaped_slice(*s);
        }
    }

    fn visit_estr_slice(&mut self) -> bool {
        do self.get::<&str> |this, s| {
            this.write_escaped_slice(*s);
        }
    }

    // Type no longer exists, vestigial function.
    fn visit_estr_fixed(&mut self, _n: uint, _sz: uint,
                        _align: uint) -> bool { fail!(); }

    fn visit_box(&mut self, mtbl: uint, inner: *TyDesc) -> bool {
        self.writer.write(['@' as u8]);
        self.write_mut_qualifier(mtbl);
        do self.get::<&raw::Box<()>> |this, b| {
            let p = ptr::to_unsafe_ptr(&b.data) as *c_void;
            this.visit_ptr_inner(p, inner);
        }
    }

    fn visit_uniq(&mut self, _mtbl: uint, inner: *TyDesc) -> bool {
        self.writer.write(['~' as u8]);
        do self.get::<*c_void> |this, b| {
            this.visit_ptr_inner(*b, inner);
        }
    }

    fn visit_uniq_managed(&mut self, _mtbl: uint, inner: *TyDesc) -> bool {
        self.writer.write(['~' as u8]);
        do self.get::<&raw::Box<()>> |this, b| {
            let p = ptr::to_unsafe_ptr(&b.data) as *c_void;
            this.visit_ptr_inner(p, inner);
        }
    }

    fn visit_ptr(&mut self, mtbl: uint, _inner: *TyDesc) -> bool {
        do self.get::<*c_void> |this, p| {
            write!(this.writer, "({} as *", *p);
            this.write_mut_qualifier(mtbl);
            this.writer.write("())".as_bytes());
        }
    }

    fn visit_rptr(&mut self, mtbl: uint, inner: *TyDesc) -> bool {
        self.writer.write(['&' as u8]);
        self.write_mut_qualifier(mtbl);
        do self.get::<*c_void> |this, p| {
            this.visit_ptr_inner(*p, inner);
        }
    }

    // Type no longer exists, vestigial function.
    fn visit_vec(&mut self, _mtbl: uint, _inner: *TyDesc) -> bool { fail!(); }

    fn visit_unboxed_vec(&mut self, mtbl: uint, inner: *TyDesc) -> bool {
        do self.get::<raw::Vec<()>> |this, b| {
            this.write_unboxed_vec_repr(mtbl, b, inner);
        }
    }

    fn visit_evec_box(&mut self, mtbl: uint, inner: *TyDesc) -> bool {
        do self.get::<&raw::Box<raw::Vec<()>>> |this, b| {
            this.writer.write(['@' as u8]);
            this.write_mut_qualifier(mtbl);
            this.write_unboxed_vec_repr(mtbl, &b.data, inner);
        }
    }

    fn visit_evec_uniq(&mut self, mtbl: uint, inner: *TyDesc) -> bool {
        do self.get::<&raw::Vec<()>> |this, b| {
            this.writer.write(['~' as u8]);
            this.write_unboxed_vec_repr(mtbl, *b, inner);
        }
    }

    fn visit_evec_uniq_managed(&mut self, mtbl: uint, inner: *TyDesc) -> bool {
        do self.get::<&raw::Box<raw::Vec<()>>> |this, b| {
            this.writer.write(['~' as u8]);
            this.write_unboxed_vec_repr(mtbl, &b.data, inner);
        }
    }

    fn visit_evec_slice(&mut self, mtbl: uint, inner: *TyDesc) -> bool {
        do self.get::<raw::Slice<()>> |this, s| {
            this.writer.write(['&' as u8]);
            this.write_mut_qualifier(mtbl);
            this.write_vec_range(mtbl, s.data, s.len, inner);
        }
    }

    fn visit_evec_fixed(&mut self, n: uint, sz: uint, _align: uint,
                        mtbl: uint, inner: *TyDesc) -> bool {
        let assumed_size = if sz == 0 { n } else { sz };
        do self.get::<()> |this, b| {
            this.write_vec_range(mtbl, ptr::to_unsafe_ptr(b), assumed_size, inner);
        }
    }

    fn visit_enter_rec(&mut self, _n_fields: uint,
                       _sz: uint, _align: uint) -> bool {
        self.writer.write(['{' as u8]);
        true
    }

    fn visit_rec_field(&mut self, i: uint, name: &str,
                       mtbl: uint, inner: *TyDesc) -> bool {
        if i != 0 {
            self.writer.write(", ".as_bytes());
        }
        self.write_mut_qualifier(mtbl);
        self.writer.write(name.as_bytes());
        self.writer.write(": ".as_bytes());
        self.visit_inner(inner);
        true
    }

    fn visit_leave_rec(&mut self, _n_fields: uint,
                       _sz: uint, _align: uint) -> bool {
        self.writer.write(['}' as u8]);
        true
    }

    fn visit_enter_class(&mut self, name: &str, named_fields: bool, n_fields: uint,
                         _sz: uint, _align: uint) -> bool {
        self.writer.write(name.as_bytes());
        if n_fields != 0 {
            if named_fields {
                self.writer.write(['{' as u8]);
            } else {
                self.writer.write(['(' as u8]);
            }
        }
        true
    }

    fn visit_class_field(&mut self, i: uint, name: &str, named: bool,
                         _mtbl: uint, inner: *TyDesc) -> bool {
        if i != 0 {
            self.writer.write(", ".as_bytes());
        }
        if named {
            self.writer.write(name.as_bytes());
            self.writer.write(": ".as_bytes());
        }
        self.visit_inner(inner);
        true
    }

    fn visit_leave_class(&mut self, _name: &str, named_fields: bool, n_fields: uint,
                         _sz: uint, _align: uint) -> bool {
        if n_fields != 0 {
            if named_fields {
                self.writer.write(['}' as u8]);
            } else {
                self.writer.write([')' as u8]);
            }
        }
        true
    }

    fn visit_enter_tup(&mut self, _n_fields: uint,
                       _sz: uint, _align: uint) -> bool {
        self.writer.write(['(' as u8]);
        true
    }

    fn visit_tup_field(&mut self, i: uint, inner: *TyDesc) -> bool {
        if i != 0 {
            self.writer.write(", ".as_bytes());
        }
        self.visit_inner(inner);
        true
    }

    fn visit_leave_tup(&mut self, _n_fields: uint,
                       _sz: uint, _align: uint) -> bool {
        if _n_fields == 1 {
            self.writer.write([',' as u8]);
        }
        self.writer.write([')' as u8]);
        true
    }

    fn visit_enter_enum(&mut self,
                        _n_variants: uint,
                        get_disr: extern unsafe fn(ptr: *Opaque) -> int,
                        _sz: uint,
                        _align: uint) -> bool {
        let disr = unsafe {
            get_disr(transmute(self.ptr))
        };
        self.var_stk.push(SearchingFor(disr));
        true
    }

    fn visit_enter_enum_variant(&mut self, _variant: uint,
                                disr_val: int,
                                n_fields: uint,
                                name: &str) -> bool {
        let mut write = false;
        match self.var_stk.pop() {
            SearchingFor(sought) => {
                if disr_val == sought {
                    self.var_stk.push(Matched);
                    write = true;
                } else {
                    self.var_stk.push(SearchingFor(sought));
                }
            }
            Matched | AlreadyFound => {
                self.var_stk.push(AlreadyFound);
            }
        }

        if write {
            self.writer.write(name.as_bytes());
            if n_fields > 0 {
                self.writer.write(['(' as u8]);
            }
        }
        true
    }

    fn visit_enum_variant_field(&mut self,
                                i: uint,
                                _offset: uint,
                                inner: *TyDesc)
                                -> bool {
        match self.var_stk[self.var_stk.len() - 1] {
            Matched => {
                if i != 0 {
                    self.writer.write(", ".as_bytes());
                }
                if ! self.visit_inner(inner) {
                    return false;
                }
            }
            _ => ()
        }
        true
    }

    fn visit_leave_enum_variant(&mut self, _variant: uint,
                                _disr_val: int,
                                n_fields: uint,
                                _name: &str) -> bool {
        match self.var_stk[self.var_stk.len() - 1] {
            Matched => {
                if n_fields > 0 {
                    self.writer.write([')' as u8]);
                }
            }
            _ => ()
        }
        true
    }

    fn visit_leave_enum(&mut self,
                        _n_variants: uint,
                        _get_disr: extern unsafe fn(ptr: *Opaque) -> int,
                        _sz: uint,
                        _align: uint)
                        -> bool {
        match self.var_stk.pop() {
            SearchingFor(*) => fail!("enum value matched no variant"),
            _ => true
        }
    }

    fn visit_enter_fn(&mut self, _purity: uint, _proto: uint,
                      _n_inputs: uint, _retstyle: uint) -> bool {
        self.writer.write("fn(".as_bytes());
        true
    }

    fn visit_fn_input(&mut self, i: uint, _mode: uint, inner: *TyDesc) -> bool {
        if i != 0 {
            self.writer.write(", ".as_bytes());
        }
        let name = unsafe { (*inner).name };
        self.writer.write(name.as_bytes());
        true
    }

    fn visit_fn_output(&mut self, _retstyle: uint, inner: *TyDesc) -> bool {
        self.writer.write(")".as_bytes());
        let name = unsafe { (*inner).name };
        if name != "()" {
            self.writer.write(" -> ".as_bytes());
            self.writer.write(name.as_bytes());
        }
        true
    }

    fn visit_leave_fn(&mut self, _purity: uint, _proto: uint,
                      _n_inputs: uint, _retstyle: uint) -> bool { true }


    fn visit_trait(&mut self, name: &str) -> bool {
        self.writer.write(name.as_bytes());
        true
    }

    fn visit_param(&mut self, _i: uint) -> bool { true }
    fn visit_self(&mut self) -> bool { true }
    fn visit_type(&mut self) -> bool { true }

    fn visit_opaque_box(&mut self) -> bool {
        self.writer.write(['@' as u8]);
        do self.get::<&raw::Box<()>> |this, b| {
            let p = ptr::to_unsafe_ptr(&b.data) as *c_void;
            this.visit_ptr_inner(p, b.type_desc);
        }
    }

    fn visit_closure_ptr(&mut self, _ck: uint) -> bool { true }
}

pub fn write_repr<T>(writer: &mut io::Writer, object: &T) {
    unsafe {
        let ptr = ptr::to_unsafe_ptr(object) as *c_void;
        let tydesc = get_tydesc::<T>();
        let u = ReprVisitor(ptr, writer);
        let mut v = reflect::MovePtrAdaptor(u);
        visit_tydesc(tydesc, &mut v as &mut TyVisitor);
    }
}

#[cfg(test)]
struct P {a: int, b: float}

#[test]
fn test_repr() {
    use prelude::*;
    use str;
    use str::Str;
    use rt::io::Decorator;
    use util::swap;
    use char::is_alphabetic;

    fn exact_test<T>(t: &T, e:&str) {
        let mut m = io::mem::MemWriter::new();
        write_repr(&mut m as &mut io::Writer, t);
        let s = str::from_utf8_owned(m.inner());
        assert_eq!(s.as_slice(), e);
    }

    exact_test(&10, "10");
    exact_test(&true, "true");
    exact_test(&false, "false");
    exact_test(&1.234, "1.234");
    exact_test(&(&"hello"), "\"hello\"");
    exact_test(&(@"hello"), "@\"hello\"");
    exact_test(&(~"he\u10f3llo"), "~\"he\\u10f3llo\"");

    exact_test(&(@10), "@10");
    exact_test(&(@mut 10), "@mut 10");
    exact_test(&((@mut 10, 2)), "(@mut 10, 2)");
    exact_test(&(~10), "~10");
    exact_test(&(&10), "&10");
    let mut x = 10;
    exact_test(&(&mut x), "&mut 10");
    exact_test(&(@mut [1, 2]), "@mut [1, 2]");

    exact_test(&(0 as *()), "(0x0 as *())");
    exact_test(&(0 as *mut ()), "(0x0 as *mut ())");

    exact_test(&(1,), "(1,)");
    exact_test(&(@[1,2,3,4,5,6,7,8]),
               "@[1, 2, 3, 4, 5, 6, 7, 8]");
    exact_test(&(@[1u8,2u8,3u8,4u8]),
               "@[1u8, 2u8, 3u8, 4u8]");
    exact_test(&(@["hi", "there"]),
               "@[\"hi\", \"there\"]");
    exact_test(&(~["hi", "there"]),
               "~[\"hi\", \"there\"]");
    exact_test(&(&["hi", "there"]),
               "&[\"hi\", \"there\"]");
    exact_test(&(P{a:10, b:1.234}),
               "repr::P{a: 10, b: 1.234}");
    exact_test(&(@P{a:10, b:1.234}),
               "@repr::P{a: 10, b: 1.234}");
    exact_test(&(~P{a:10, b:1.234}),
               "~repr::P{a: 10, b: 1.234}");
    exact_test(&(10u8, ~"hello"),
               "(10u8, ~\"hello\")");
    exact_test(&(10u16, ~"hello"),
               "(10u16, ~\"hello\")");
    exact_test(&(10u32, ~"hello"),
               "(10u32, ~\"hello\")");
    exact_test(&(10u64, ~"hello"),
               "(10u64, ~\"hello\")");

    exact_test(&(&[1, 2]), "&[1, 2]");
    exact_test(&(&mut [1, 2]), "&mut [1, 2]");

    exact_test(&'\'', "'\\''");
    exact_test(&'"', "'\"'");
    exact_test(&("'"), "\"'\"");
    exact_test(&("\""), "\"\\\"\"");

    exact_test(&println, "fn(&str)");
    exact_test(&swap::<int>, "fn(&mut int, &mut int)");
    exact_test(&is_alphabetic, "fn(char) -> bool");
    exact_test(&(~5 as ~ToStr), "~to_str::ToStr:Send");

    struct Foo;
    exact_test(&(~[Foo, Foo]), "~[repr::test_repr::Foo, repr::test_repr::Foo]");

    struct Bar(int, int);
    exact_test(&(Bar(2, 2)), "repr::test_repr::Bar(2, 2)");
}
