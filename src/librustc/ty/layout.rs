// Copyright 2016 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

pub use self::Integer::*;
pub use self::Primitive::*;

use session::{self, DataTypeKind, Session};
use ty::{self, Ty, TyCtxt, TypeFoldable, ReprOptions, ReprFlags};

use syntax::ast::{self, FloatTy, IntTy, UintTy};
use syntax::attr;
use syntax_pos::DUMMY_SP;

use std::cmp;
use std::fmt;
use std::i128;
use std::iter;
use std::mem;
use std::ops::{Add, Sub, Mul, AddAssign, Deref, RangeInclusive};

use ich::StableHashingContext;
use rustc_data_structures::stable_hasher::{HashStable, StableHasher,
                                           StableHasherResult};

/// Parsed [Data layout](http://llvm.org/docs/LangRef.html#data-layout)
/// for a target, which contains everything needed to compute layouts.
pub struct TargetDataLayout {
    pub endian: Endian,
    pub i1_align: Align,
    pub i8_align: Align,
    pub i16_align: Align,
    pub i32_align: Align,
    pub i64_align: Align,
    pub i128_align: Align,
    pub f32_align: Align,
    pub f64_align: Align,
    pub pointer_size: Size,
    pub pointer_align: Align,
    pub aggregate_align: Align,

    /// Alignments for vector types.
    pub vector_align: Vec<(Size, Align)>
}

impl Default for TargetDataLayout {
    /// Creates an instance of `TargetDataLayout`.
    fn default() -> TargetDataLayout {
        TargetDataLayout {
            endian: Endian::Big,
            i1_align: Align::from_bits(8, 8).unwrap(),
            i8_align: Align::from_bits(8, 8).unwrap(),
            i16_align: Align::from_bits(16, 16).unwrap(),
            i32_align: Align::from_bits(32, 32).unwrap(),
            i64_align: Align::from_bits(32, 64).unwrap(),
            i128_align: Align::from_bits(32, 64).unwrap(),
            f32_align: Align::from_bits(32, 32).unwrap(),
            f64_align: Align::from_bits(64, 64).unwrap(),
            pointer_size: Size::from_bits(64),
            pointer_align: Align::from_bits(64, 64).unwrap(),
            aggregate_align: Align::from_bits(0, 64).unwrap(),
            vector_align: vec![
                (Size::from_bits(64), Align::from_bits(64, 64).unwrap()),
                (Size::from_bits(128), Align::from_bits(128, 128).unwrap())
            ]
        }
    }
}

impl TargetDataLayout {
    pub fn parse(sess: &Session) -> TargetDataLayout {
        // Parse a bit count from a string.
        let parse_bits = |s: &str, kind: &str, cause: &str| {
            s.parse::<u64>().unwrap_or_else(|err| {
                sess.err(&format!("invalid {} `{}` for `{}` in \"data-layout\": {}",
                                  kind, s, cause, err));
                0
            })
        };

        // Parse a size string.
        let size = |s: &str, cause: &str| {
            Size::from_bits(parse_bits(s, "size", cause))
        };

        // Parse an alignment string.
        let align = |s: &[&str], cause: &str| {
            if s.is_empty() {
                sess.err(&format!("missing alignment for `{}` in \"data-layout\"", cause));
            }
            let abi = parse_bits(s[0], "alignment", cause);
            let pref = s.get(1).map_or(abi, |pref| parse_bits(pref, "alignment", cause));
            Align::from_bits(abi, pref).unwrap_or_else(|err| {
                sess.err(&format!("invalid alignment for `{}` in \"data-layout\": {}",
                                  cause, err));
                Align::from_bits(8, 8).unwrap()
            })
        };

        let mut dl = TargetDataLayout::default();
        let mut i128_align_src = 64;
        for spec in sess.target.target.data_layout.split("-") {
            match &spec.split(":").collect::<Vec<_>>()[..] {
                &["e"] => dl.endian = Endian::Little,
                &["E"] => dl.endian = Endian::Big,
                &["a", ref a..] => dl.aggregate_align = align(a, "a"),
                &["f32", ref a..] => dl.f32_align = align(a, "f32"),
                &["f64", ref a..] => dl.f64_align = align(a, "f64"),
                &[p @ "p", s, ref a..] | &[p @ "p0", s, ref a..] => {
                    dl.pointer_size = size(s, p);
                    dl.pointer_align = align(a, p);
                }
                &[s, ref a..] if s.starts_with("i") => {
                    let bits = match s[1..].parse::<u64>() {
                        Ok(bits) => bits,
                        Err(_) => {
                            size(&s[1..], "i"); // For the user error.
                            continue;
                        }
                    };
                    let a = align(a, s);
                    match bits {
                        1 => dl.i1_align = a,
                        8 => dl.i8_align = a,
                        16 => dl.i16_align = a,
                        32 => dl.i32_align = a,
                        64 => dl.i64_align = a,
                        _ => {}
                    }
                    if bits >= i128_align_src && bits <= 128 {
                        // Default alignment for i128 is decided by taking the alignment of
                        // largest-sized i{64...128}.
                        i128_align_src = bits;
                        dl.i128_align = a;
                    }
                }
                &[s, ref a..] if s.starts_with("v") => {
                    let v_size = size(&s[1..], "v");
                    let a = align(a, s);
                    if let Some(v) = dl.vector_align.iter_mut().find(|v| v.0 == v_size) {
                        v.1 = a;
                        continue;
                    }
                    // No existing entry, add a new one.
                    dl.vector_align.push((v_size, a));
                }
                _ => {} // Ignore everything else.
            }
        }

        // Perform consistency checks against the Target information.
        let endian_str = match dl.endian {
            Endian::Little => "little",
            Endian::Big => "big"
        };
        if endian_str != sess.target.target.target_endian {
            sess.err(&format!("inconsistent target specification: \"data-layout\" claims \
                               architecture is {}-endian, while \"target-endian\" is `{}`",
                              endian_str, sess.target.target.target_endian));
        }

        if dl.pointer_size.bits().to_string() != sess.target.target.target_pointer_width {
            sess.err(&format!("inconsistent target specification: \"data-layout\" claims \
                               pointers are {}-bit, while \"target-pointer-width\" is `{}`",
                              dl.pointer_size.bits(), sess.target.target.target_pointer_width));
        }

        dl
    }

    /// Return exclusive upper bound on object size.
    ///
    /// The theoretical maximum object size is defined as the maximum positive `isize` value.
    /// This ensures that the `offset` semantics remain well-defined by allowing it to correctly
    /// index every address within an object along with one byte past the end, along with allowing
    /// `isize` to store the difference between any two pointers into an object.
    ///
    /// The upper bound on 64-bit currently needs to be lower because LLVM uses a 64-bit integer
    /// to represent object size in bits. It would need to be 1 << 61 to account for this, but is
    /// currently conservatively bounded to 1 << 47 as that is enough to cover the current usable
    /// address space on 64-bit ARMv8 and x86_64.
    pub fn obj_size_bound(&self) -> u64 {
        match self.pointer_size.bits() {
            16 => 1 << 15,
            32 => 1 << 31,
            64 => 1 << 47,
            bits => bug!("obj_size_bound: unknown pointer bit size {}", bits)
        }
    }

    pub fn ptr_sized_integer(&self) -> Integer {
        match self.pointer_size.bits() {
            16 => I16,
            32 => I32,
            64 => I64,
            bits => bug!("ptr_sized_integer: unknown pointer bit size {}", bits)
        }
    }

    pub fn vector_align(&self, vec_size: Size) -> Align {
        for &(size, align) in &self.vector_align {
            if size == vec_size {
                return align;
            }
        }
        // Default to natural alignment, which is what LLVM does.
        // That is, use the size, rounded up to a power of 2.
        let align = vec_size.bytes().next_power_of_two();
        Align::from_bytes(align, align).unwrap()
    }
}

pub trait HasDataLayout: Copy {
    fn data_layout(&self) -> &TargetDataLayout;
}

impl<'a> HasDataLayout for &'a TargetDataLayout {
    fn data_layout(&self) -> &TargetDataLayout {
        self
    }
}

/// Endianness of the target, which must match cfg(target-endian).
#[derive(Copy, Clone)]
pub enum Endian {
    Little,
    Big
}

/// Size of a type in bytes.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Size {
    raw: u64
}

impl Size {
    pub fn from_bits(bits: u64) -> Size {
        // Avoid potential overflow from `bits + 7`.
        Size::from_bytes(bits / 8 + ((bits % 8) + 7) / 8)
    }

    pub fn from_bytes(bytes: u64) -> Size {
        if bytes >= (1 << 61) {
            bug!("Size::from_bytes: {} bytes in bits doesn't fit in u64", bytes)
        }
        Size {
            raw: bytes
        }
    }

    pub fn bytes(self) -> u64 {
        self.raw
    }

    pub fn bits(self) -> u64 {
        self.bytes() * 8
    }

    pub fn abi_align(self, align: Align) -> Size {
        let mask = align.abi() - 1;
        Size::from_bytes((self.bytes() + mask) & !mask)
    }

    pub fn is_abi_aligned(self, align: Align) -> bool {
        let mask = align.abi() - 1;
        self.bytes() & mask == 0
    }

    pub fn checked_add<C: HasDataLayout>(self, offset: Size, cx: C) -> Option<Size> {
        let dl = cx.data_layout();

        // Each Size is less than dl.obj_size_bound(), so the sum is
        // also less than 1 << 62 (and therefore can't overflow).
        let bytes = self.bytes() + offset.bytes();

        if bytes < dl.obj_size_bound() {
            Some(Size::from_bytes(bytes))
        } else {
            None
        }
    }

    pub fn checked_mul<C: HasDataLayout>(self, count: u64, cx: C) -> Option<Size> {
        let dl = cx.data_layout();

        match self.bytes().checked_mul(count) {
            Some(bytes) if bytes < dl.obj_size_bound() => {
                Some(Size::from_bytes(bytes))
            }
            _ => None
        }
    }
}

// Panicking addition, subtraction and multiplication for convenience.
// Avoid during layout computation, return `LayoutError` instead.

impl Add for Size {
    type Output = Size;
    fn add(self, other: Size) -> Size {
        // Each Size is less than 1 << 61, so the sum is
        // less than 1 << 62 (and therefore can't overflow).
        Size::from_bytes(self.bytes() + other.bytes())
    }
}

impl Sub for Size {
    type Output = Size;
    fn sub(self, other: Size) -> Size {
        // Each Size is less than 1 << 61, so an underflow
        // would result in a value larger than 1 << 61,
        // which Size::from_bytes will catch for us.
        Size::from_bytes(self.bytes() - other.bytes())
    }
}

impl Mul<u64> for Size {
    type Output = Size;
    fn mul(self, count: u64) -> Size {
        match self.bytes().checked_mul(count) {
            Some(bytes) => Size::from_bytes(bytes),
            None => {
                bug!("Size::mul: {} * {} doesn't fit in u64", self.bytes(), count)
            }
        }
    }
}

impl AddAssign for Size {
    fn add_assign(&mut self, other: Size) {
        *self = *self + other;
    }
}

/// Alignment of a type in bytes, both ABI-mandated and preferred.
/// Each field is a power of two, giving the alignment a maximum
/// value of 2^(2^8 - 1), which is limited by LLVM to a i32, with
/// a maximum capacity of 2^31 - 1 or 2147483647.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Align {
    abi: u8,
    pref: u8,
}

impl Align {
    pub fn from_bits(abi: u64, pref: u64) -> Result<Align, String> {
        Align::from_bytes(Size::from_bits(abi).bytes(),
                          Size::from_bits(pref).bytes())
    }

    pub fn from_bytes(abi: u64, pref: u64) -> Result<Align, String> {
        let log2 = |align: u64| {
            // Treat an alignment of 0 bytes like 1-byte alignment.
            if align == 0 {
                return Ok(0);
            }

            let mut bytes = align;
            let mut pow: u8 = 0;
            while (bytes & 1) == 0 {
                pow += 1;
                bytes >>= 1;
            }
            if bytes != 1 {
                Err(format!("`{}` is not a power of 2", align))
            } else if pow > 30 {
                Err(format!("`{}` is too large", align))
            } else {
                Ok(pow)
            }
        };

        Ok(Align {
            abi: log2(abi)?,
            pref: log2(pref)?,
        })
    }

    pub fn abi(self) -> u64 {
        1 << self.abi
    }

    pub fn pref(self) -> u64 {
        1 << self.pref
    }

    pub fn abi_bits(self) -> u64 {
        self.abi() * 8
    }

    pub fn pref_bits(self) -> u64 {
        self.pref() * 8
    }

    pub fn min(self, other: Align) -> Align {
        Align {
            abi: cmp::min(self.abi, other.abi),
            pref: cmp::min(self.pref, other.pref),
        }
    }

    pub fn max(self, other: Align) -> Align {
        Align {
            abi: cmp::max(self.abi, other.abi),
            pref: cmp::max(self.pref, other.pref),
        }
    }
}

/// Integers, also used for enum discriminants.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Integer {
    I1,
    I8,
    I16,
    I32,
    I64,
    I128,
}

impl<'a, 'tcx> Integer {
    pub fn size(&self) -> Size {
        match *self {
            I1 => Size::from_bits(1),
            I8 => Size::from_bytes(1),
            I16 => Size::from_bytes(2),
            I32 => Size::from_bytes(4),
            I64  => Size::from_bytes(8),
            I128  => Size::from_bytes(16),
        }
    }

    pub fn align<C: HasDataLayout>(&self, cx: C) -> Align {
        let dl = cx.data_layout();

        match *self {
            I1 => dl.i1_align,
            I8 => dl.i8_align,
            I16 => dl.i16_align,
            I32 => dl.i32_align,
            I64 => dl.i64_align,
            I128 => dl.i128_align,
        }
    }

    pub fn to_ty(&self, tcx: TyCtxt<'a, 'tcx, 'tcx>, signed: bool) -> Ty<'tcx> {
        match (*self, signed) {
            (I1, false) => tcx.types.u8,
            (I8, false) => tcx.types.u8,
            (I16, false) => tcx.types.u16,
            (I32, false) => tcx.types.u32,
            (I64, false) => tcx.types.u64,
            (I128, false) => tcx.types.u128,
            (I1, true) => tcx.types.i8,
            (I8, true) => tcx.types.i8,
            (I16, true) => tcx.types.i16,
            (I32, true) => tcx.types.i32,
            (I64, true) => tcx.types.i64,
            (I128, true) => tcx.types.i128,
        }
    }

    /// Find the smallest Integer type which can represent the signed value.
    pub fn fit_signed(x: i128) -> Integer {
        match x {
            -0x0000_0000_0000_0001...0x0000_0000_0000_0000 => I1,
            -0x0000_0000_0000_0080...0x0000_0000_0000_007f => I8,
            -0x0000_0000_0000_8000...0x0000_0000_0000_7fff => I16,
            -0x0000_0000_8000_0000...0x0000_0000_7fff_ffff => I32,
            -0x8000_0000_0000_0000...0x7fff_ffff_ffff_ffff => I64,
            _ => I128
        }
    }

    /// Find the smallest Integer type which can represent the unsigned value.
    pub fn fit_unsigned(x: u128) -> Integer {
        match x {
            0...0x0000_0000_0000_0001 => I1,
            0...0x0000_0000_0000_00ff => I8,
            0...0x0000_0000_0000_ffff => I16,
            0...0x0000_0000_ffff_ffff => I32,
            0...0xffff_ffff_ffff_ffff => I64,
            _ => I128,
        }
    }

    /// Find the smallest integer with the given alignment.
    pub fn for_abi_align<C: HasDataLayout>(cx: C, align: Align) -> Option<Integer> {
        let dl = cx.data_layout();

        let wanted = align.abi();
        for &candidate in &[I8, I16, I32, I64, I128] {
            let ty = Int(candidate, false);
            if wanted == ty.align(dl).abi() && wanted == ty.size(dl).bytes() {
                return Some(candidate);
            }
        }
        None
    }

    /// Get the Integer type from an attr::IntType.
    pub fn from_attr<C: HasDataLayout>(cx: C, ity: attr::IntType) -> Integer {
        let dl = cx.data_layout();

        match ity {
            attr::SignedInt(IntTy::I8) | attr::UnsignedInt(UintTy::U8) => I8,
            attr::SignedInt(IntTy::I16) | attr::UnsignedInt(UintTy::U16) => I16,
            attr::SignedInt(IntTy::I32) | attr::UnsignedInt(UintTy::U32) => I32,
            attr::SignedInt(IntTy::I64) | attr::UnsignedInt(UintTy::U64) => I64,
            attr::SignedInt(IntTy::I128) | attr::UnsignedInt(UintTy::U128) => I128,
            attr::SignedInt(IntTy::Is) | attr::UnsignedInt(UintTy::Us) => {
                dl.ptr_sized_integer()
            }
        }
    }

    /// Find the appropriate Integer type and signedness for the given
    /// signed discriminant range and #[repr] attribute.
    /// N.B.: u128 values above i128::MAX will be treated as signed, but
    /// that shouldn't affect anything, other than maybe debuginfo.
    fn repr_discr(tcx: TyCtxt<'a, 'tcx, 'tcx>,
                  ty: Ty<'tcx>,
                  repr: &ReprOptions,
                  min: i128,
                  max: i128)
                  -> (Integer, bool) {
        // Theoretically, negative values could be larger in unsigned representation
        // than the unsigned representation of the signed minimum. However, if there
        // are any negative values, the only valid unsigned representation is u128
        // which can fit all i128 values, so the result remains unaffected.
        let unsigned_fit = Integer::fit_unsigned(cmp::max(min as u128, max as u128));
        let signed_fit = cmp::max(Integer::fit_signed(min), Integer::fit_signed(max));

        let mut min_from_extern = None;
        let min_default = I8;

        if let Some(ity) = repr.int {
            let discr = Integer::from_attr(tcx, ity);
            let fit = if ity.is_signed() { signed_fit } else { unsigned_fit };
            if discr < fit {
                bug!("Integer::repr_discr: `#[repr]` hint too small for \
                  discriminant range of enum `{}", ty)
            }
            return (discr, ity.is_signed());
        }

        if repr.c() {
            match &tcx.sess.target.target.arch[..] {
                // WARNING: the ARM EABI has two variants; the one corresponding
                // to `at_least == I32` appears to be used on Linux and NetBSD,
                // but some systems may use the variant corresponding to no
                // lower bound.  However, we don't run on those yet...?
                "arm" => min_from_extern = Some(I32),
                _ => min_from_extern = Some(I32),
            }
        }

        let at_least = min_from_extern.unwrap_or(min_default);

        // If there are no negative values, we can use the unsigned fit.
        if min >= 0 {
            (cmp::max(unsigned_fit, at_least), false)
        } else {
            (cmp::max(signed_fit, at_least), true)
        }
    }
}

/// Fundamental unit of memory access and layout.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum Primitive {
    /// The `bool` is the signedness of the `Integer` type.
    ///
    /// One would think we would not care about such details this low down,
    /// but some ABIs are described in terms of C types and ISAs where the
    /// integer arithmetic is done on {sign,zero}-extended registers, e.g.
    /// a negative integer passed by zero-extension will appear positive in
    /// the callee, and most operations on it will produce the wrong values.
    Int(Integer, bool),
    F32,
    F64,
    Pointer
}

impl<'a, 'tcx> Primitive {
    pub fn size<C: HasDataLayout>(self, cx: C) -> Size {
        let dl = cx.data_layout();

        match self {
            Int(i, _) => i.size(),
            F32 => Size::from_bits(32),
            F64 => Size::from_bits(64),
            Pointer => dl.pointer_size
        }
    }

    pub fn align<C: HasDataLayout>(self, cx: C) -> Align {
        let dl = cx.data_layout();

        match self {
            Int(i, _) => i.align(dl),
            F32 => dl.f32_align,
            F64 => dl.f64_align,
            Pointer => dl.pointer_align
        }
    }

    pub fn to_ty(&self, tcx: TyCtxt<'a, 'tcx, 'tcx>) -> Ty<'tcx> {
        match *self {
            Int(i, signed) => i.to_ty(tcx, signed),
            F32 => tcx.types.f32,
            F64 => tcx.types.f64,
            Pointer => tcx.mk_mut_ptr(tcx.mk_nil()),
        }
    }
}

/// The first half of a fat pointer.
/// - For a trait object, this is the address of the box.
/// - For a slice, this is the base address.
pub const FAT_PTR_ADDR: usize = 0;

/// The second half of a fat pointer.
/// - For a trait object, this is the address of the vtable.
/// - For a slice, this is the length.
pub const FAT_PTR_EXTRA: usize = 1;

/// Describes how the fields of a type are located in memory.
#[derive(PartialEq, Eq, Hash, Debug)]
pub enum FieldPlacement {
    /// All fields start at no offset. The `usize` is the field count.
    Union(usize),

    /// Array/vector-like placement, with all fields of identical types.
    Array {
        stride: Size,
        count: u64
    },

    /// Struct-like placement, with precomputed offsets.
    ///
    /// Fields are guaranteed to not overlap, but note that gaps
    /// before, between and after all the fields are NOT always
    /// padding, and as such their contents may not be discarded.
    /// For example, enum variants leave a gap at the start,
    /// where the discriminant field in the enum layout goes.
    Arbitrary {
        /// Offsets for the first byte of each field,
        /// ordered to match the source definition order.
        /// This vector does not go in increasing order.
        // FIXME(eddyb) use small vector optimization for the common case.
        offsets: Vec<Size>,

        /// Maps source order field indices to memory order indices,
        /// depending how fields were permuted.
        // FIXME(camlorn) also consider small vector  optimization here.
        memory_index: Vec<u32>
    }
}

impl FieldPlacement {
    pub fn count(&self) -> usize {
        match *self {
            FieldPlacement::Union(count) => count,
            FieldPlacement::Array { count, .. } => {
                let usize_count = count as usize;
                assert_eq!(usize_count as u64, count);
                usize_count
            }
            FieldPlacement::Arbitrary { ref offsets, .. } => offsets.len()
        }
    }

    pub fn offset(&self, i: usize) -> Size {
        match *self {
            FieldPlacement::Union(_) => Size::from_bytes(0),
            FieldPlacement::Array { stride, count } => {
                let i = i as u64;
                assert!(i < count);
                stride * i
            }
            FieldPlacement::Arbitrary { ref offsets, .. } => offsets[i]
        }
    }

    pub fn memory_index(&self, i: usize) -> usize {
        match *self {
            FieldPlacement::Union(_) |
            FieldPlacement::Array { .. } => i,
            FieldPlacement::Arbitrary { ref memory_index, .. } => {
                let r = memory_index[i];
                assert_eq!(r as usize as u32, r);
                r as usize
            }
        }
    }

    /// Get source indices of the fields by increasing offsets.
    #[inline]
    pub fn index_by_increasing_offset<'a>(&'a self) -> impl iter::Iterator<Item=usize>+'a {
        let mut inverse_small = [0u8; 64];
        let mut inverse_big = vec![];
        let use_small = self.count() <= inverse_small.len();

        // We have to write this logic twice in order to keep the array small.
        if let FieldPlacement::Arbitrary { ref memory_index, .. } = *self {
            if use_small {
                for i in 0..self.count() {
                    inverse_small[memory_index[i] as usize] = i as u8;
                }
            } else {
                inverse_big = vec![0; self.count()];
                for i in 0..self.count() {
                    inverse_big[memory_index[i] as usize] = i as u32;
                }
            }
        }

        (0..self.count()).map(move |i| {
            match *self {
                FieldPlacement::Union(_) |
                FieldPlacement::Array { .. } => i,
                FieldPlacement::Arbitrary { .. } => {
                    if use_small { inverse_small[i] as usize }
                    else { inverse_big[i] as usize }
                }
            }
        })
    }
}

/// Describes how values of the type are passed by target ABIs,
/// in terms of categories of C types there are ABI rules for.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum Abi {
    Scalar(Primitive),
    Vector {
        element: Primitive,
        count: u64
    },
    Aggregate {
        /// If true, the size is exact, otherwise it's only a lower bound.
        sized: bool,
        packed: bool
    }
}

impl Abi {
    /// Returns true if the layout corresponds to an unsized type.
    pub fn is_unsized(&self) -> bool {
        match *self {
            Abi::Scalar(_) | Abi::Vector { .. } => false,
            Abi::Aggregate { sized, .. } => !sized
        }
    }

    /// Returns true if the fields of the layout are packed.
    pub fn is_packed(&self) -> bool {
        match *self {
            Abi::Scalar(_) | Abi::Vector { .. } => false,
            Abi::Aggregate { packed, .. } => packed
        }
    }
}

#[derive(PartialEq, Eq, Hash, Debug)]
pub enum Variants {
    /// Single enum variants, structs/tuples, unions, and all non-ADTs.
    Single {
        index: usize
    },

    /// General-case enums: for each case there is a struct, and they all have
    /// all space reserved for the discriminant, and their first field starts
    /// at a non-0 offset, after where the discriminant would go.
    Tagged {
        discr: Primitive,
        /// Inclusive wrap-around range of discriminant values, that is,
        /// if min > max, it represents min..=u128::MAX followed by 0..=max.
        // FIXME(eddyb) always use the shortest range, e.g. by finding
        // the largest space between two consecutive discriminants and
        // taking everything else as the (shortest) discriminant range.
        discr_range: RangeInclusive<u128>,
        variants: Vec<CachedLayout>,
    },

    /// Two cases distinguished by a niche (a value invalid for a type):
    /// the variant `dataful_variant` contains a niche at an arbitrary
    /// offset (field 0 of the enum), which is set to `niche_value`
    /// for the other variant.
    ///
    /// For example, `Option<(usize, &T)>`  is represented such that
    /// `None` has a null pointer for the second tuple field, and
    /// `Some` is the identity function (with a non-null reference).
    NicheFilling {
        dataful_variant: usize,
        niche: Primitive,
        niche_value: u128,
        variants: Vec<CachedLayout>,
    }
}

#[derive(Copy, Clone, Debug)]
pub enum LayoutError<'tcx> {
    Unknown(Ty<'tcx>),
    SizeOverflow(Ty<'tcx>)
}

impl<'tcx> fmt::Display for LayoutError<'tcx> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            LayoutError::Unknown(ty) => {
                write!(f, "the type `{:?}` has an unknown layout", ty)
            }
            LayoutError::SizeOverflow(ty) => {
                write!(f, "the type `{:?}` is too big for the current architecture", ty)
            }
        }
    }
}

#[derive(PartialEq, Eq, Hash, Debug)]
pub struct CachedLayout {
    pub variants: Variants,
    pub fields: FieldPlacement,
    pub abi: Abi,
    pub align: Align,
    pub primitive_align: Align,
    pub size: Size
}

fn layout_raw<'a, 'tcx>(tcx: TyCtxt<'a, 'tcx, 'tcx>,
                        query: ty::ParamEnvAnd<'tcx, Ty<'tcx>>)
                        -> Result<&'tcx CachedLayout, LayoutError<'tcx>>
{
    let (param_env, ty) = query.into_parts();

    let rec_limit = tcx.sess.recursion_limit.get();
    let depth = tcx.layout_depth.get();
    if depth > rec_limit {
        tcx.sess.fatal(
            &format!("overflow representing the type `{}`", ty));
    }

    tcx.layout_depth.set(depth+1);
    let layout = CachedLayout::compute_uncached(tcx, param_env, ty);
    tcx.layout_depth.set(depth);

    layout
}

pub fn provide(providers: &mut ty::maps::Providers) {
    *providers = ty::maps::Providers {
        layout_raw,
        ..*providers
    };
}

impl<'a, 'tcx> CachedLayout {
    fn compute_uncached(tcx: TyCtxt<'a, 'tcx, 'tcx>,
                        param_env: ty::ParamEnv<'tcx>,
                        ty: Ty<'tcx>)
                        -> Result<&'tcx Self, LayoutError<'tcx>> {
        let cx = (tcx, param_env);
        let dl = cx.data_layout();
        let scalar = |value: Primitive| {
            let align = value.align(dl);
            tcx.intern_layout(CachedLayout {
                variants: Variants::Single { index: 0 },
                fields: FieldPlacement::Union(0),
                abi: Abi::Scalar(value),
                size: value.size(dl),
                align,
                primitive_align: align
            })
        };
        #[derive(Copy, Clone, Debug)]
        enum StructKind {
            /// A tuple, closure, or univariant which cannot be coerced to unsized.
            AlwaysSized,
            /// A univariant, the last field of which may be coerced to unsized.
            MaybeUnsized,
            /// A univariant, but part of an enum.
            EnumVariant(Integer),
        }
        let univariant_uninterned = |fields: &[TyLayout], repr: &ReprOptions, kind| {
            let packed = repr.packed();
            if packed && repr.align > 0 {
                bug!("struct cannot be packed and aligned");
            }

            let mut align = if packed {
                dl.i8_align
            } else {
                dl.aggregate_align
            };

            let mut primitive_align = align;
            let mut sized = true;

            // Anything with repr(C) or repr(packed) doesn't optimize.
            // Neither do  1-member and 2-member structs.
            // In addition, code in trans assume that 2-element structs can become pairs.
            // It's easier to just short-circuit here.
            let (mut optimize, sort_ascending) = match kind {
                StructKind::AlwaysSized |
                StructKind::MaybeUnsized => (fields.len() > 2, false),
                StructKind::EnumVariant(discr) => {
                    (discr.size().bytes() == 1, true)
                }
            };

            optimize &= (repr.flags & ReprFlags::IS_UNOPTIMISABLE).is_empty();

            let mut offsets = vec![Size::from_bytes(0); fields.len()];
            let mut inverse_memory_index: Vec<u32> = (0..fields.len() as u32).collect();

            if optimize {
                let end = if let StructKind::MaybeUnsized = kind {
                    fields.len() - 1
                } else {
                    fields.len()
                };
                if end > 0 {
                    let optimizing  = &mut inverse_memory_index[..end];
                    if sort_ascending {
                        optimizing.sort_by_key(|&x| fields[x as usize].align.abi());
                    } else {
                        optimizing.sort_by(| &a, &b | {
                            let a = fields[a as usize].align.abi();
                            let b = fields[b as usize].align.abi();
                            b.cmp(&a)
                        });
                    }
                }
            }

            // inverse_memory_index holds field indices by increasing memory offset.
            // That is, if field 5 has offset 0, the first element of inverse_memory_index is 5.
            // We now write field offsets to the corresponding offset slot;
            // field 5 with offset 0 puts 0 in offsets[5].
            // At the bottom of this function, we use inverse_memory_index to produce memory_index.

            let mut offset = Size::from_bytes(0);

            if let StructKind::EnumVariant(discr) = kind {
                offset = discr.size();
                if !packed {
                    let discr_align = discr.align(dl);
                    align = align.max(discr_align);
                    primitive_align = primitive_align.max(discr_align);
                }
            }

            for i in inverse_memory_index.iter() {
                let field = fields[*i as usize];
                if !sized {
                    bug!("univariant: field #{} of `{}` comes after unsized field",
                        offsets.len(), ty);
                }

                if field.is_unsized() {
                    sized = false;
                }

                // Invariant: offset < dl.obj_size_bound() <= 1<<61
                if !packed {
                    offset = offset.abi_align(field.align);
                    align = align.max(field.align);
                    primitive_align = primitive_align.max(field.primitive_align);
                }

                debug!("univariant offset: {:?} field: {:#?}", offset, field);
                offsets[*i as usize] = offset;

                offset = offset.checked_add(field.size, dl)
                    .ok_or(LayoutError::SizeOverflow(ty))?;
            }

            if repr.align > 0 {
                let repr_align = repr.align as u64;
                align = align.max(Align::from_bytes(repr_align, repr_align).unwrap());
                debug!("univariant repr_align: {:?}", repr_align);
            }

            debug!("univariant min_size: {:?}", offset);
            let min_size = offset;

            // As stated above, inverse_memory_index holds field indices by increasing offset.
            // This makes it an already-sorted view of the offsets vec.
            // To invert it, consider:
            // If field 5 has offset 0, offsets[0] is 5, and memory_index[5] should be 0.
            // Field 5 would be the first element, so memory_index is i:
            // Note: if we didn't optimize, it's already right.

            let mut memory_index;
            if optimize {
                memory_index = vec![0; inverse_memory_index.len()];

                for i in 0..inverse_memory_index.len() {
                    memory_index[inverse_memory_index[i] as usize]  = i as u32;
                }
            } else {
                memory_index = inverse_memory_index;
            }

            Ok(CachedLayout {
                variants: Variants::Single { index: 0 },
                fields: FieldPlacement::Arbitrary {
                    offsets,
                    memory_index
                },
                abi: Abi::Aggregate {
                    sized,
                    packed
                },
                align,
                primitive_align,
                size: min_size.abi_align(align)
            })
        };
        let univariant = |fields: &[TyLayout], repr: &ReprOptions, kind| {
            Ok(tcx.intern_layout(univariant_uninterned(fields, repr, kind)?))
        };
        assert!(!ty.has_infer_types());

        let ptr_layout = |pointee: Ty<'tcx>| {
            let pointee = tcx.normalize_associated_type_in_env(&pointee, param_env);
            if pointee.is_sized(tcx, param_env, DUMMY_SP) {
                return Ok(scalar(Pointer));
            }

            let unsized_part = tcx.struct_tail(pointee);
            let metadata = match unsized_part.sty {
                ty::TyForeign(..) => return Ok(scalar(Pointer)),
                ty::TySlice(_) | ty::TyStr => {
                    Int(dl.ptr_sized_integer(), false)
                }
                ty::TyDynamic(..) => Pointer,
                _ => return Err(LayoutError::Unknown(unsized_part))
            };

            // Effectively a (ptr, meta) tuple.
            let align = Pointer.align(dl).max(metadata.align(dl));
            let meta_offset = Pointer.size(dl);
            assert_eq!(meta_offset, meta_offset.abi_align(metadata.align(dl)));
            let fields = FieldPlacement::Arbitrary {
                offsets: vec![Size::from_bytes(0), meta_offset],
                memory_index: vec![0, 1]
            };
            Ok(tcx.intern_layout(CachedLayout {
                variants: Variants::Single { index: 0 },
                fields,
                abi: Abi::Aggregate {
                    sized: true,
                    packed: false
                },
                align,
                primitive_align: align,
                size: (meta_offset + metadata.size(dl)).abi_align(align)
            }))
        };

        Ok(match ty.sty {
            // Basic scalars.
            ty::TyBool => scalar(Int(I1, false)),
            ty::TyChar => scalar(Int(I32, false)),
            ty::TyInt(ity) => {
                scalar(Int(Integer::from_attr(dl, attr::SignedInt(ity)), true))
            }
            ty::TyUint(ity) => {
                scalar(Int(Integer::from_attr(dl, attr::UnsignedInt(ity)), false))
            }
            ty::TyFloat(FloatTy::F32) => scalar(F32),
            ty::TyFloat(FloatTy::F64) => scalar(F64),
            ty::TyFnPtr(_) => scalar(Pointer),

            // The never type.
            ty::TyNever => {
                univariant(&[], &ReprOptions::default(), StructKind::AlwaysSized)?
            }

            // Potentially-fat pointers.
            ty::TyRef(_, ty::TypeAndMut { ty: pointee, .. }) |
            ty::TyRawPtr(ty::TypeAndMut { ty: pointee, .. }) => {
                ptr_layout(pointee)?
            }
            ty::TyAdt(def, _) if def.is_box() => {
                ptr_layout(ty.boxed_ty())?
            }

            // Arrays and slices.
            ty::TyArray(element, mut count) => {
                if count.has_projections() {
                    count = tcx.normalize_associated_type_in_env(&count, param_env);
                    if count.has_projections() {
                        return Err(LayoutError::Unknown(ty));
                    }
                }

                let element = cx.layout_of(element)?;
                let count = count.val.to_const_int().unwrap().to_u64().unwrap();
                let size = element.size.checked_mul(count, dl)
                    .ok_or(LayoutError::SizeOverflow(ty))?;

                tcx.intern_layout(CachedLayout {
                    variants: Variants::Single { index: 0 },
                    fields: FieldPlacement::Array {
                        stride: element.size,
                        count
                    },
                    abi: Abi::Aggregate {
                        sized: true,
                        packed: false
                    },
                    align: element.align,
                    primitive_align: element.primitive_align,
                    size
                })
            }
            ty::TySlice(element) => {
                let element = cx.layout_of(element)?;
                tcx.intern_layout(CachedLayout {
                    variants: Variants::Single { index: 0 },
                    fields: FieldPlacement::Array {
                        stride: element.size,
                        count: 0
                    },
                    abi: Abi::Aggregate {
                        sized: false,
                        packed: false
                    },
                    align: element.align,
                    primitive_align: element.primitive_align,
                    size: Size::from_bytes(0)
                })
            }
            ty::TyStr => {
                tcx.intern_layout(CachedLayout {
                    variants: Variants::Single { index: 0 },
                    fields: FieldPlacement::Array {
                        stride: Size::from_bytes(1),
                        count: 0
                    },
                    abi: Abi::Aggregate {
                        sized: false,
                        packed: false
                    },
                    align: dl.i8_align,
                    primitive_align: dl.i8_align,
                    size: Size::from_bytes(0)
                })
            }

            // Odd unit types.
            ty::TyFnDef(..) => {
                univariant(&[], &ReprOptions::default(), StructKind::AlwaysSized)?
            }
            ty::TyDynamic(..) | ty::TyForeign(..) => {
                let mut unit = univariant_uninterned(&[], &ReprOptions::default(),
                  StructKind::AlwaysSized)?;
                match unit.abi {
                    Abi::Aggregate { ref mut sized, .. } => *sized = false,
                    _ => bug!()
                }
                tcx.intern_layout(unit)
            }

            // Tuples, generators and closures.
            ty::TyGenerator(def_id, ref substs, _) => {
                let tys = substs.field_tys(def_id, tcx);
                univariant(&tys.map(|ty| cx.layout_of(ty)).collect::<Result<Vec<_>, _>>()?,
                    &ReprOptions::default(),
                    StructKind::AlwaysSized)?
            }

            ty::TyClosure(def_id, ref substs) => {
                let tys = substs.upvar_tys(def_id, tcx);
                univariant(&tys.map(|ty| cx.layout_of(ty)).collect::<Result<Vec<_>, _>>()?,
                    &ReprOptions::default(),
                    StructKind::AlwaysSized)?
            }

            ty::TyTuple(tys, _) => {
                let kind = if tys.len() == 0 {
                    StructKind::AlwaysSized
                } else {
                    StructKind::MaybeUnsized
                };

                univariant(&tys.iter().map(|ty| cx.layout_of(ty)).collect::<Result<Vec<_>, _>>()?,
                    &ReprOptions::default(), kind)?
            }

            // SIMD vector types.
            ty::TyAdt(def, ..) if def.repr.simd() => {
                let count = ty.simd_size(tcx) as u64;
                let element = cx.layout_of(ty.simd_type(tcx))?;
                let element_scalar = match element.abi {
                    Abi::Scalar(value) => value,
                    _ => {
                        tcx.sess.fatal(&format!("monomorphising SIMD type `{}` with \
                                                a non-machine element type `{}`",
                                                ty, element.ty));
                    }
                };
                let size = element.size.checked_mul(count, dl)
                    .ok_or(LayoutError::SizeOverflow(ty))?;
                let align = dl.vector_align(size);
                let size = size.abi_align(align);

                tcx.intern_layout(CachedLayout {
                    variants: Variants::Single { index: 0 },
                    fields: FieldPlacement::Array {
                        stride: element.size,
                        count
                    },
                    abi: Abi::Vector {
                        element: element_scalar,
                        count
                    },
                    size,
                    align,
                    primitive_align: align
                })
            }

            // ADTs.
            ty::TyAdt(def, substs) => {
                // Cache the field layouts.
                let variants = def.variants.iter().map(|v| {
                    v.fields.iter().map(|field| {
                        cx.layout_of(field.ty(tcx, substs))
                    }).collect::<Result<Vec<_>, _>>()
                }).collect::<Result<Vec<_>, _>>()?;

                if variants.is_empty() {
                    // Uninhabitable; represent as unit
                    // (Typechecking will reject discriminant-sizing attrs.)

                    return univariant(&[], &def.repr, StructKind::AlwaysSized);
                }

                if def.is_union() {
                    let packed = def.repr.packed();
                    if packed && def.repr.align > 0 {
                        bug!("Union cannot be packed and aligned");
                    }

                    let mut primitive_align = if def.repr.packed() {
                        dl.i8_align
                    } else {
                        dl.aggregate_align
                    };

                    let mut align = if def.repr.align > 0 {
                        let repr_align = def.repr.align as u64;
                        primitive_align.max(
                            Align::from_bytes(repr_align, repr_align).unwrap())
                    } else {
                        primitive_align
                    };

                    let mut size = Size::from_bytes(0);
                    for field in &variants[0] {
                        assert!(!field.is_unsized());

                        if !packed {
                            align = align.max(field.align);
                            primitive_align = primitive_align.max(field.primitive_align);
                        }
                        size = cmp::max(size, field.size);
                    }

                    return Ok(tcx.intern_layout(CachedLayout {
                        variants: Variants::Single { index: 0 },
                        fields: FieldPlacement::Union(variants[0].len()),
                        abi: Abi::Aggregate {
                            sized: true,
                            packed
                        },
                        align,
                        primitive_align,
                        size: size.abi_align(align)
                    }));
                }

                if !def.is_enum() || (variants.len() == 1 &&
                                      !def.repr.inhibit_enum_layout_opt() &&
                                      !variants[0].is_empty()) {
                    // Struct, or union, or univariant enum equivalent to a struct.
                    // (Typechecking will reject discriminant-sizing attrs.)

                    let kind = if def.is_enum() || variants[0].len() == 0 {
                        StructKind::AlwaysSized
                    } else {
                        let param_env = tcx.param_env(def.did);
                        let last_field = def.variants[0].fields.last().unwrap();
                        let always_sized = tcx.type_of(last_field.did)
                          .is_sized(tcx, param_env, DUMMY_SP);
                        if !always_sized { StructKind::MaybeUnsized }
                        else { StructKind::AlwaysSized }
                    };

                    return univariant(&variants[0], &def.repr, kind);
                }

                let no_explicit_discriminants = def.variants.iter().enumerate()
                    .all(|(i, v)| v.discr == ty::VariantDiscr::Relative(i));

                if variants.len() == 2 &&
                   !def.repr.inhibit_enum_layout_opt() &&
                   no_explicit_discriminants {
                    // Nullable pointer optimization
                    for i in 0..2 {
                        if !variants[1 - i].iter().all(|f| f.is_zst()) {
                            continue;
                        }

                        for (field_index, field) in variants[i].iter().enumerate() {
                            if let Some((offset, niche, niche_value)) = field.find_niche(cx)? {
                                let mut st = vec![
                                    univariant_uninterned(&variants[0],
                                        &def.repr, StructKind::AlwaysSized)?,
                                    univariant_uninterned(&variants[1],
                                        &def.repr, StructKind::AlwaysSized)?
                                ];
                                for (i, v) in st.iter_mut().enumerate() {
                                    v.variants = Variants::Single { index: i };
                                }
                                let offset = st[i].fields.offset(field_index) + offset;
                                let CachedLayout {
                                    mut abi,
                                    size,
                                    mut align,
                                    mut primitive_align,
                                    ..
                                } = st[i];

                                let mut niche_align = niche.align(dl);
                                if offset.bytes() == 0 && niche.size(dl) == size {
                                    abi = Abi::Scalar(niche);
                                } else if let Abi::Aggregate { ref mut packed, .. } = abi {
                                    if offset.abi_align(niche_align) != offset {
                                        *packed = true;
                                        niche_align = dl.i8_align;
                                    }
                                }
                                align = align.max(niche_align);
                                primitive_align = primitive_align.max(niche_align);

                                return Ok(tcx.intern_layout(CachedLayout {
                                    variants: Variants::NicheFilling {
                                        dataful_variant: i,
                                        niche,
                                        niche_value,
                                        variants: st,
                                    },
                                    fields: FieldPlacement::Arbitrary {
                                        offsets: vec![offset],
                                        memory_index: vec![0]
                                    },
                                    abi,
                                    size,
                                    align,
                                    primitive_align
                                }));
                            }
                        }
                    }
                }

                let (mut min, mut max) = (i128::max_value(), i128::min_value());
                for discr in def.discriminants(tcx) {
                    let x = discr.to_u128_unchecked() as i128;
                    if x < min { min = x; }
                    if x > max { max = x; }
                }
                let (min_ity, signed) = Integer::repr_discr(tcx, ty, &def.repr, min, max);

                let mut align = dl.aggregate_align;
                let mut primitive_align = dl.aggregate_align;
                let mut size = Size::from_bytes(0);

                // We're interested in the smallest alignment, so start large.
                let mut start_align = Align::from_bytes(256, 256).unwrap();
                assert_eq!(Integer::for_abi_align(dl, start_align), None);

                // Create the set of structs that represent each variant.
                let mut variants = variants.into_iter().enumerate().map(|(i, field_layouts)| {
                    let mut st = univariant_uninterned(&field_layouts,
                        &def.repr, StructKind::EnumVariant(min_ity))?;
                    st.variants = Variants::Single { index: i };
                    // Find the first field we can't move later
                    // to make room for a larger discriminant.
                    for field in st.fields.index_by_increasing_offset().map(|j| field_layouts[j]) {
                        if !field.is_zst() || field.align.abi() != 1 {
                            start_align = start_align.min(field.align);
                            break;
                        }
                    }
                    size = cmp::max(size, st.size);
                    align = align.max(st.align);
                    primitive_align = primitive_align.max(st.primitive_align);
                    Ok(st)
                }).collect::<Result<Vec<_>, _>>()?;

                // Align the maximum variant size to the largest alignment.
                size = size.abi_align(align);

                if size.bytes() >= dl.obj_size_bound() {
                    return Err(LayoutError::SizeOverflow(ty));
                }

                let typeck_ity = Integer::from_attr(dl, def.repr.discr_type());
                if typeck_ity < min_ity {
                    // It is a bug if Layout decided on a greater discriminant size than typeck for
                    // some reason at this point (based on values discriminant can take on). Mostly
                    // because this discriminant will be loaded, and then stored into variable of
                    // type calculated by typeck. Consider such case (a bug): typeck decided on
                    // byte-sized discriminant, but layout thinks we need a 16-bit to store all
                    // discriminant values. That would be a bug, because then, in trans, in order
                    // to store this 16-bit discriminant into 8-bit sized temporary some of the
                    // space necessary to represent would have to be discarded (or layout is wrong
                    // on thinking it needs 16 bits)
                    bug!("layout decided on a larger discriminant type ({:?}) than typeck ({:?})",
                         min_ity, typeck_ity);
                    // However, it is fine to make discr type however large (as an optimisation)
                    // after this point – we’ll just truncate the value we load in trans.
                }

                // Check to see if we should use a different type for the
                // discriminant. We can safely use a type with the same size
                // as the alignment of the first field of each variant.
                // We increase the size of the discriminant to avoid LLVM copying
                // padding when it doesn't need to. This normally causes unaligned
                // load/stores and excessive memcpy/memset operations. By using a
                // bigger integer size, LLVM can be sure about it's contents and
                // won't be so conservative.

                // Use the initial field alignment
                let mut ity = Integer::for_abi_align(dl, start_align).unwrap_or(min_ity);

                // If the alignment is not larger than the chosen discriminant size,
                // don't use the alignment as the final size.
                if ity <= min_ity {
                    ity = min_ity;
                } else {
                    // Patch up the variants' first few fields.
                    let old_ity_size = min_ity.size();
                    let new_ity_size = ity.size();
                    for variant in &mut variants {
                        match variant.fields {
                            FieldPlacement::Arbitrary { ref mut offsets, .. } => {
                                for i in offsets {
                                    if *i <= old_ity_size {
                                        assert_eq!(*i, old_ity_size);
                                        *i = new_ity_size;
                                    }
                                }
                                // We might be making the struct larger.
                                if variant.size <= old_ity_size {
                                    variant.size = new_ity_size;
                                }
                            }
                            _ => bug!()
                        }
                    }
                }

                let discr = Int(ity, signed);
                tcx.intern_layout(CachedLayout {
                    variants: Variants::Tagged {
                        discr,
                        discr_range: (min as u128)..=(max as u128),
                        variants
                    },
                    // FIXME(eddyb): using `FieldPlacement::Arbitrary` here results
                    // in lost optimizations, specifically around allocations, see
                    // `test/codegen/{alloc-optimisation,vec-optimizes-away}.rs`.
                    fields: FieldPlacement::Union(1),
                    abi: if discr.size(dl) == size {
                        Abi::Scalar(discr)
                    } else {
                        Abi::Aggregate {
                            sized: true,
                            packed: false
                        }
                    },
                    align,
                    primitive_align,
                    size
                })
            }

            // Types with no meaningful known layout.
            ty::TyProjection(_) | ty::TyAnon(..) => {
                let normalized = tcx.normalize_associated_type_in_env(&ty, param_env);
                if ty == normalized {
                    return Err(LayoutError::Unknown(ty));
                }
                tcx.layout_raw(param_env.and(normalized))?
            }
            ty::TyParam(_) => {
                return Err(LayoutError::Unknown(ty));
            }
            ty::TyInfer(_) | ty::TyError => {
                bug!("CachedLayout::compute: unexpected type `{}`", ty)
            }
        })
    }

    /// This is invoked by the `layout_raw` query to record the final
    /// layout of each type.
    #[inline]
    fn record_layout_for_printing(tcx: TyCtxt<'a, 'tcx, 'tcx>,
                                  ty: Ty<'tcx>,
                                  param_env: ty::ParamEnv<'tcx>,
                                  layout: TyLayout<'tcx>) {
        // If we are running with `-Zprint-type-sizes`, record layouts for
        // dumping later. Ignore layouts that are done with non-empty
        // environments or non-monomorphic layouts, as the user only wants
        // to see the stuff resulting from the final trans session.
        if
            !tcx.sess.opts.debugging_opts.print_type_sizes ||
            ty.has_param_types() ||
            ty.has_self_ty() ||
            !param_env.caller_bounds.is_empty()
        {
            return;
        }

        Self::record_layout_for_printing_outlined(tcx, ty, param_env, layout)
    }

    fn record_layout_for_printing_outlined(tcx: TyCtxt<'a, 'tcx, 'tcx>,
                                           ty: Ty<'tcx>,
                                           param_env: ty::ParamEnv<'tcx>,
                                           layout: TyLayout<'tcx>) {
        let cx = (tcx, param_env);
        // (delay format until we actually need it)
        let record = |kind, opt_discr_size, variants| {
            let type_desc = format!("{:?}", ty);
            tcx.sess.code_stats.borrow_mut().record_type_size(kind,
                                                              type_desc,
                                                              layout.align,
                                                              layout.size,
                                                              opt_discr_size,
                                                              variants);
        };

        let adt_def = match ty.sty {
            ty::TyAdt(ref adt_def, _) => {
                debug!("print-type-size t: `{:?}` process adt", ty);
                adt_def
            }

            ty::TyClosure(..) => {
                debug!("print-type-size t: `{:?}` record closure", ty);
                record(DataTypeKind::Closure, None, vec![]);
                return;
            }

            _ => {
                debug!("print-type-size t: `{:?}` skip non-nominal", ty);
                return;
            }
        };

        let adt_kind = adt_def.adt_kind();

        let build_variant_info = |n: Option<ast::Name>,
                                  flds: &[ast::Name],
                                  layout: TyLayout<'tcx>| {
            let mut min_size = Size::from_bytes(0);
            let field_info: Vec<_> = flds.iter().enumerate().map(|(i, &name)| {
                match layout.field(cx, i) {
                    Err(err) => {
                        bug!("no layout found for field {}: `{:?}`", name, err);
                    }
                    Ok(field_layout) => {
                        let offset = layout.fields.offset(i);
                        let field_end = offset + field_layout.size;
                        if min_size < field_end {
                            min_size = field_end;
                        }
                        session::FieldInfo {
                            name: name.to_string(),
                            offset: offset.bytes(),
                            size: field_layout.size.bytes(),
                            align: field_layout.align.abi(),
                        }
                    }
                }
            }).collect();

            session::VariantInfo {
                name: n.map(|n|n.to_string()),
                kind: if layout.is_unsized() {
                    session::SizeKind::Min
                } else {
                    session::SizeKind::Exact
                },
                align: layout.align.abi(),
                size: if min_size.bytes() == 0 {
                    layout.size.bytes()
                } else {
                    min_size.bytes()
                },
                fields: field_info,
            }
        };

        match layout.variants {
            Variants::Single { .. } => {
                let variant_names = || {
                    adt_def.variants.iter().map(|v|format!("{}", v.name)).collect::<Vec<_>>()
                };
                debug!("print-type-size `{:#?}` variants: {:?}",
                       layout, variant_names());
                assert!(adt_def.variants.len() <= 1,
                        "univariant with variants {:?}", variant_names());
                if adt_def.variants.len() == 1 {
                    let variant_def = &adt_def.variants[0];
                    let fields: Vec<_> =
                        variant_def.fields.iter().map(|f| f.name).collect();
                    record(adt_kind.into(),
                           None,
                           vec![build_variant_info(Some(variant_def.name),
                                                   &fields,
                                                   layout)]);
                } else {
                    // (This case arises for *empty* enums; so give it
                    // zero variants.)
                    record(adt_kind.into(), None, vec![]);
                }
            }

            Variants::NicheFilling { .. } |
            Variants::Tagged { .. } => {
                debug!("print-type-size `{:#?}` adt general variants def {}",
                       ty, adt_def.variants.len());
                let variant_infos: Vec<_> =
                    adt_def.variants.iter().enumerate().map(|(i, variant_def)| {
                        let fields: Vec<_> =
                            variant_def.fields.iter().map(|f| f.name).collect();
                        build_variant_info(Some(variant_def.name),
                                            &fields,
                                            layout.for_variant(i))
                    })
                    .collect();
                record(adt_kind.into(), match layout.variants {
                    Variants::Tagged { discr, .. } => Some(discr.size(tcx)),
                    _ => None
                }, variant_infos);
            }
        }
    }
}

/// Type size "skeleton", i.e. the only information determining a type's size.
/// While this is conservative, (aside from constant sizes, only pointers,
/// newtypes thereof and null pointer optimized enums are allowed), it is
/// enough to statically check common usecases of transmute.
#[derive(Copy, Clone, Debug)]
pub enum SizeSkeleton<'tcx> {
    /// Any statically computable Layout.
    Known(Size),

    /// A potentially-fat pointer.
    Pointer {
        /// If true, this pointer is never null.
        non_zero: bool,
        /// The type which determines the unsized metadata, if any,
        /// of this pointer. Either a type parameter or a projection
        /// depending on one, with regions erased.
        tail: Ty<'tcx>
    }
}

impl<'a, 'tcx> SizeSkeleton<'tcx> {
    pub fn compute(ty: Ty<'tcx>,
                   tcx: TyCtxt<'a, 'tcx, 'tcx>,
                   param_env: ty::ParamEnv<'tcx>)
                   -> Result<SizeSkeleton<'tcx>, LayoutError<'tcx>> {
        assert!(!ty.has_infer_types());

        // First try computing a static layout.
        let err = match (tcx, param_env).layout_of(ty) {
            Ok(layout) => {
                return Ok(SizeSkeleton::Known(layout.size));
            }
            Err(err) => err
        };

        let ptr_skeleton = |pointee: Ty<'tcx>| {
            let non_zero = !ty.is_unsafe_ptr();
            let tail = tcx.struct_tail(pointee);
            match tail.sty {
                ty::TyParam(_) | ty::TyProjection(_) => {
                    assert!(tail.has_param_types() || tail.has_self_ty());
                    Ok(SizeSkeleton::Pointer {
                        non_zero,
                        tail: tcx.erase_regions(&tail)
                    })
                }
                _ => {
                    bug!("SizeSkeleton::compute({}): layout errored ({}), yet \
                            tail `{}` is not a type parameter or a projection",
                            ty, err, tail)
                }
            }
        };

        match ty.sty {
            ty::TyRef(_, ty::TypeAndMut { ty: pointee, .. }) |
            ty::TyRawPtr(ty::TypeAndMut { ty: pointee, .. }) => {
                ptr_skeleton(pointee)
            }
            ty::TyAdt(def, _) if def.is_box() => {
                ptr_skeleton(ty.boxed_ty())
            }

            ty::TyAdt(def, substs) => {
                // Only newtypes and enums w/ nullable pointer optimization.
                if def.is_union() || def.variants.is_empty() || def.variants.len() > 2 {
                    return Err(err);
                }

                // Get a zero-sized variant or a pointer newtype.
                let zero_or_ptr_variant = |i: usize| {
                    let fields = def.variants[i].fields.iter().map(|field| {
                        SizeSkeleton::compute(field.ty(tcx, substs), tcx, param_env)
                    });
                    let mut ptr = None;
                    for field in fields {
                        let field = field?;
                        match field {
                            SizeSkeleton::Known(size) => {
                                if size.bytes() > 0 {
                                    return Err(err);
                                }
                            }
                            SizeSkeleton::Pointer {..} => {
                                if ptr.is_some() {
                                    return Err(err);
                                }
                                ptr = Some(field);
                            }
                        }
                    }
                    Ok(ptr)
                };

                let v0 = zero_or_ptr_variant(0)?;
                // Newtype.
                if def.variants.len() == 1 {
                    if let Some(SizeSkeleton::Pointer { non_zero, tail }) = v0 {
                        return Ok(SizeSkeleton::Pointer {
                            non_zero: non_zero ||
                                Some(def.did) == tcx.lang_items().non_zero(),
                            tail,
                        });
                    } else {
                        return Err(err);
                    }
                }

                let v1 = zero_or_ptr_variant(1)?;
                // Nullable pointer enum optimization.
                match (v0, v1) {
                    (Some(SizeSkeleton::Pointer { non_zero: true, tail }), None) |
                    (None, Some(SizeSkeleton::Pointer { non_zero: true, tail })) => {
                        Ok(SizeSkeleton::Pointer {
                            non_zero: false,
                            tail,
                        })
                    }
                    _ => Err(err)
                }
            }

            ty::TyProjection(_) | ty::TyAnon(..) => {
                let normalized = tcx.normalize_associated_type_in_env(&ty, param_env);
                if ty == normalized {
                    Err(err)
                } else {
                    SizeSkeleton::compute(normalized, tcx, param_env)
                }
            }

            _ => Err(err)
        }
    }

    pub fn same_size(self, other: SizeSkeleton) -> bool {
        match (self, other) {
            (SizeSkeleton::Known(a), SizeSkeleton::Known(b)) => a == b,
            (SizeSkeleton::Pointer { tail: a, .. },
             SizeSkeleton::Pointer { tail: b, .. }) => a == b,
            _ => false
        }
    }
}

/// The details of the layout of a type, alongside the type itself.
/// Provides various type traversal APIs (e.g. recursing into fields).
///
/// Note that the details are NOT guaranteed to always be identical
/// to those obtained from `layout_of(ty)`, as we need to produce
/// layouts for which Rust types do not exist, such as enum variants
/// or synthetic fields of enums (i.e. discriminants) and fat pointers.
#[derive(Copy, Clone, Debug)]
pub struct TyLayout<'tcx> {
    pub ty: Ty<'tcx>,
    cached: &'tcx CachedLayout
}

impl<'tcx> Deref for TyLayout<'tcx> {
    type Target = &'tcx CachedLayout;
    fn deref(&self) -> &&'tcx CachedLayout {
        &self.cached
    }
}

pub trait HasTyCtxt<'tcx>: HasDataLayout {
    fn tcx<'a>(&'a self) -> TyCtxt<'a, 'tcx, 'tcx>;
}

impl<'a, 'gcx, 'tcx> HasDataLayout for TyCtxt<'a, 'gcx, 'tcx> {
    fn data_layout(&self) -> &TargetDataLayout {
        &self.data_layout
    }
}

impl<'a, 'gcx, 'tcx> HasTyCtxt<'gcx> for TyCtxt<'a, 'gcx, 'tcx> {
    fn tcx<'b>(&'b self) -> TyCtxt<'b, 'gcx, 'gcx> {
        self.global_tcx()
    }
}

impl<'a, 'gcx, 'tcx, T: Copy> HasDataLayout for (TyCtxt<'a, 'gcx, 'tcx>, T) {
    fn data_layout(&self) -> &TargetDataLayout {
        self.0.data_layout()
    }
}

impl<'a, 'gcx, 'tcx, T: Copy> HasTyCtxt<'gcx> for (TyCtxt<'a, 'gcx, 'tcx>, T) {
    fn tcx<'b>(&'b self) -> TyCtxt<'b, 'gcx, 'gcx> {
        self.0.tcx()
    }
}

pub trait MaybeResult<T> {
    fn map_same<F: FnOnce(T) -> T>(self, f: F) -> Self;
}

impl<T> MaybeResult<T> for T {
    fn map_same<F: FnOnce(T) -> T>(self, f: F) -> Self {
        f(self)
    }
}

impl<T, E> MaybeResult<T> for Result<T, E> {
    fn map_same<F: FnOnce(T) -> T>(self, f: F) -> Self {
        self.map(f)
    }
}

pub trait LayoutOf<T> {
    type TyLayout;

    fn layout_of(self, ty: T) -> Self::TyLayout;
}

impl<'a, 'tcx> LayoutOf<Ty<'tcx>> for (TyCtxt<'a, 'tcx, 'tcx>, ty::ParamEnv<'tcx>) {
    type TyLayout = Result<TyLayout<'tcx>, LayoutError<'tcx>>;

    /// Computes the layout of a type. Note that this implicitly
    /// executes in "reveal all" mode.
    #[inline]
    fn layout_of(self, ty: Ty<'tcx>) -> Self::TyLayout {
        let (tcx, param_env) = self;

        let ty = tcx.normalize_associated_type_in_env(&ty, param_env.reveal_all());
        let cached = tcx.layout_raw(param_env.reveal_all().and(ty))?;
        let layout = TyLayout {
            ty,
            cached
        };

        // NB: This recording is normally disabled; when enabled, it
        // can however trigger recursive invocations of `layout_of`.
        // Therefore, we execute it *after* the main query has
        // completed, to avoid problems around recursive structures
        // and the like. (Admitedly, I wasn't able to reproduce a problem
        // here, but it seems like the right thing to do. -nmatsakis)
        CachedLayout::record_layout_for_printing(tcx, ty, param_env, layout);

        Ok(layout)
    }
}

impl<'a, 'tcx> LayoutOf<Ty<'tcx>> for (ty::maps::TyCtxtAt<'a, 'tcx, 'tcx>,
                                       ty::ParamEnv<'tcx>) {
    type TyLayout = Result<TyLayout<'tcx>, LayoutError<'tcx>>;

    /// Computes the layout of a type. Note that this implicitly
    /// executes in "reveal all" mode.
    #[inline]
    fn layout_of(self, ty: Ty<'tcx>) -> Self::TyLayout {
        let (tcx_at, param_env) = self;

        let ty = tcx_at.tcx.normalize_associated_type_in_env(&ty, param_env.reveal_all());
        let cached = tcx_at.layout_raw(param_env.reveal_all().and(ty))?;
        let layout = TyLayout {
            ty,
            cached
        };

        // NB: This recording is normally disabled; when enabled, it
        // can however trigger recursive invocations of `layout_of`.
        // Therefore, we execute it *after* the main query has
        // completed, to avoid problems around recursive structures
        // and the like. (Admitedly, I wasn't able to reproduce a problem
        // here, but it seems like the right thing to do. -nmatsakis)
        CachedLayout::record_layout_for_printing(tcx_at.tcx, ty, param_env, layout);

        Ok(layout)
    }
}

impl<'a, 'tcx> TyLayout<'tcx> {
    pub fn for_variant(&self, variant_index: usize) -> Self {
        let cached = match self.variants {
            Variants::Single { .. } => self.cached,

            Variants::NicheFilling { ref variants, .. } |
            Variants::Tagged { ref variants, .. } => {
                &variants[variant_index]
            }
        };
        assert_eq!(cached.variants, Variants::Single { index: variant_index });

        TyLayout {
            ty: self.ty,
            cached
        }
    }

    pub fn field<C>(&self, cx: C, i: usize) -> C::TyLayout
        where C: LayoutOf<Ty<'tcx>> + HasTyCtxt<'tcx>,
              C::TyLayout: MaybeResult<TyLayout<'tcx>>
    {
        let tcx = cx.tcx();
        let ptr_field_layout = |pointee: Ty<'tcx>| {
            assert!(i < 2);

            // Reuse the fat *T type as its own thin pointer data field.
            // This provides information about e.g. DST struct pointees
            // (which may have no non-DST form), and will work as long
            // as the `Abi` or `FieldPlacement` is checked by users.
            if i == 0 {
                return cx.layout_of(Pointer.to_ty(tcx)).map_same(|mut ptr_layout| {
                    ptr_layout.ty = self.ty;
                    ptr_layout
                });
            }

            let meta_ty = match tcx.struct_tail(pointee).sty {
                ty::TySlice(_) |
                ty::TyStr => tcx.types.usize,
                ty::TyDynamic(..) => {
                    // FIXME(eddyb) use an usize/fn() array with
                    // the correct number of vtables slots.
                    tcx.mk_imm_ref(tcx.types.re_static, tcx.mk_nil())
                }
                _ => bug!("TyLayout::field_type({:?}): not applicable", self)
            };
            cx.layout_of(meta_ty)
        };

        cx.layout_of(match self.ty.sty {
            ty::TyBool |
            ty::TyChar |
            ty::TyInt(_) |
            ty::TyUint(_) |
            ty::TyFloat(_) |
            ty::TyFnPtr(_) |
            ty::TyNever |
            ty::TyFnDef(..) |
            ty::TyDynamic(..) |
            ty::TyForeign(..) => {
                bug!("TyLayout::field_type({:?}): not applicable", self)
            }

            // Potentially-fat pointers.
            ty::TyRef(_, ty::TypeAndMut { ty: pointee, .. }) |
            ty::TyRawPtr(ty::TypeAndMut { ty: pointee, .. }) => {
                return ptr_field_layout(pointee);
            }
            ty::TyAdt(def, _) if def.is_box() => {
                return ptr_field_layout(self.ty.boxed_ty());
            }

            // Arrays and slices.
            ty::TyArray(element, _) |
            ty::TySlice(element) => element,
            ty::TyStr => tcx.types.u8,

            // Tuples, generators and closures.
            ty::TyClosure(def_id, ref substs) => {
                substs.upvar_tys(def_id, tcx).nth(i).unwrap()
            }

            ty::TyGenerator(def_id, ref substs, _) => {
                substs.field_tys(def_id, tcx).nth(i).unwrap()
            }

            ty::TyTuple(tys, _) => tys[i],

            // SIMD vector types.
            ty::TyAdt(def, ..) if def.repr.simd() => {
                self.ty.simd_type(tcx)
            }

            // ADTs.
            ty::TyAdt(def, substs) => {
                match self.variants {
                    Variants::Single { index } => {
                        let mut field_ty = def.variants[index].fields[i].ty(tcx, substs);

                        // Treat NonZero<*T> as containing &T.
                        // This is especially useful for fat pointers.
                        if Some(def.did) == tcx.lang_items().non_zero() {
                            if let ty::TyRawPtr(mt) = field_ty.sty {
                                field_ty = tcx.mk_ref(tcx.types.re_erased, mt);
                            }
                        }

                        field_ty
                    }

                    // Discriminant field for enums (where applicable).
                    Variants::Tagged { discr, .. } |
                    Variants::NicheFilling { niche: discr, .. } => {
                        return cx.layout_of([discr.to_ty(tcx)][i]);
                    }
                }
            }

            ty::TyProjection(_) | ty::TyAnon(..) | ty::TyParam(_) |
            ty::TyInfer(_) | ty::TyError => {
                bug!("TyLayout::field_type: unexpected type `{}`", self.ty)
            }
        })
    }

    /// Returns true if the layout corresponds to an unsized type.
    pub fn is_unsized(&self) -> bool {
        self.abi.is_unsized()
    }

    /// Returns true if the fields of the layout are packed.
    pub fn is_packed(&self) -> bool {
        self.abi.is_packed()
    }

    /// Returns true if the type is a ZST and not unsized.
    pub fn is_zst(&self) -> bool {
        match self.abi {
            Abi::Scalar(_) => false,
            Abi::Vector { count, .. } => count == 0,
            Abi::Aggregate { sized, .. } => sized && self.size.bytes() == 0
        }
    }

    pub fn size_and_align(&self) -> (Size, Align) {
        (self.size, self.align)
    }

    /// Find the offset of a niche leaf field, starting from
    /// the given type and recursing through aggregates.
    /// The tuple is `(offset, primitive, niche_value)`.
    // FIXME(eddyb) track value ranges and traverse already optimized enums.
    fn find_niche<C>(&self, cx: C)
        -> Result<Option<(Size, Primitive, u128)>, LayoutError<'tcx>>
        where C: LayoutOf<Ty<'tcx>, TyLayout = Result<Self, LayoutError<'tcx>>> +
                 HasTyCtxt<'tcx>
    {
        let tcx = cx.tcx();
        match (&self.variants, self.abi, &self.ty.sty) {
            // FIXME(eddyb) check this via value ranges on scalars.
            (_, Abi::Scalar(Int(I1, _)), _) => {
                Ok(Some((Size::from_bytes(0), Int(I8, false), 2)))
            }
            (_, Abi::Scalar(Int(I32, _)), &ty::TyChar) => {
                Ok(Some((Size::from_bytes(0), Int(I32, false), 0x10FFFF+1)))
            }
            (_, Abi::Scalar(Pointer), &ty::TyRef(..)) |
            (_, Abi::Scalar(Pointer), &ty::TyFnPtr(..)) => {
                Ok(Some((Size::from_bytes(0), Pointer, 0)))
            }
            (_, Abi::Scalar(Pointer), &ty::TyAdt(def, _)) if def.is_box() => {
                Ok(Some((Size::from_bytes(0), Pointer, 0)))
            }

            // FIXME(eddyb) check this via value ranges on scalars.
            (&Variants::Tagged { discr, ref discr_range, .. }, _, _) => {
                // FIXME(eddyb) support negative/wrap-around discriminant ranges.
                if discr_range.start < discr_range.end {
                    if discr_range.start > 0 {
                        Ok(Some((self.fields.offset(0), discr, 0)))
                    } else {
                        let bits = discr.size(tcx).bits();
                        assert!(bits <= 128);
                        let max_value = !0u128 >> (128 - bits);
                        if discr_range.end < max_value {
                            Ok(Some((self.fields.offset(0), discr, discr_range.end + 1)))
                        } else {
                            Ok(None)
                        }
                    }
                } else {
                    Ok(None)
                }
            }

            // Is this the NonZero lang item wrapping a pointer or integer type?
            (_, _, &ty::TyAdt(def, _)) if Some(def.did) == tcx.lang_items().non_zero() => {
                let field = self.field(cx, 0)?;
                let offset = self.fields.offset(0);
                if let Abi::Scalar(value) = field.abi {
                    Ok(Some((offset, value, 0)))
                } else {
                    Ok(None)
                }
            }

            // Perhaps one of the fields is non-zero, let's recurse and find out.
            _ => {
                if let FieldPlacement::Array { count, .. } = self.fields {
                    if count > 0 {
                        return self.field(cx, 0)?.find_niche(cx);
                    }
                }
                for i in 0..self.fields.count() {
                    let r = self.field(cx, i)?.find_niche(cx)?;
                    if let Some((offset, primitive, niche_value)) = r {
                        let offset = self.fields.offset(i) + offset;
                        return Ok(Some((offset, primitive, niche_value)));
                    }
                }
                Ok(None)
            }
        }
    }
}

impl<'gcx> HashStable<StableHashingContext<'gcx>> for Variants {
    fn hash_stable<W: StableHasherResult>(&self,
                                          hcx: &mut StableHashingContext<'gcx>,
                                          hasher: &mut StableHasher<W>) {
        use ty::layout::Variants::*;
        mem::discriminant(self).hash_stable(hcx, hasher);

        match *self {
            Single { index } => {
                index.hash_stable(hcx, hasher);
            }
            Tagged {
                discr,
                discr_range: RangeInclusive { start, end },
                ref variants,
            } => {
                discr.hash_stable(hcx, hasher);
                start.hash_stable(hcx, hasher);
                end.hash_stable(hcx, hasher);
                variants.hash_stable(hcx, hasher);
            }
            NicheFilling {
                dataful_variant,
                ref niche,
                niche_value,
                ref variants,
            } => {
                dataful_variant.hash_stable(hcx, hasher);
                niche.hash_stable(hcx, hasher);
                niche_value.hash_stable(hcx, hasher);
                variants.hash_stable(hcx, hasher);
            }
        }
    }
}

impl<'gcx> HashStable<StableHashingContext<'gcx>> for FieldPlacement {
    fn hash_stable<W: StableHasherResult>(&self,
                                          hcx: &mut StableHashingContext<'gcx>,
                                          hasher: &mut StableHasher<W>) {
        use ty::layout::FieldPlacement::*;
        mem::discriminant(self).hash_stable(hcx, hasher);

        match *self {
            Union(count) => {
                count.hash_stable(hcx, hasher);
            }
            Array { count, stride } => {
                count.hash_stable(hcx, hasher);
                stride.hash_stable(hcx, hasher);
            }
            Arbitrary { ref offsets, ref memory_index } => {
                offsets.hash_stable(hcx, hasher);
                memory_index.hash_stable(hcx, hasher);
            }
        }
    }
}

impl<'gcx> HashStable<StableHashingContext<'gcx>> for Abi {
    fn hash_stable<W: StableHasherResult>(&self,
                                          hcx: &mut StableHashingContext<'gcx>,
                                          hasher: &mut StableHasher<W>) {
        use ty::layout::Abi::*;
        mem::discriminant(self).hash_stable(hcx, hasher);

        match *self {
            Scalar(ref value) => {
                value.hash_stable(hcx, hasher);
            }
            Vector { ref element, count } => {
                element.hash_stable(hcx, hasher);
                count.hash_stable(hcx, hasher);
            }
            Aggregate { packed, sized } => {
                packed.hash_stable(hcx, hasher);
                sized.hash_stable(hcx, hasher);
            }
        }
    }
}

impl_stable_hash_for!(struct ::ty::layout::CachedLayout {
    variants,
    fields,
    abi,
    size,
    align,
    primitive_align
});

impl_stable_hash_for!(enum ::ty::layout::Integer {
    I1,
    I8,
    I16,
    I32,
    I64,
    I128
});

impl_stable_hash_for!(enum ::ty::layout::Primitive {
    Int(integer, signed),
    F32,
    F64,
    Pointer
});

impl_stable_hash_for!(struct ::ty::layout::Align {
    abi,
    pref
});

impl_stable_hash_for!(struct ::ty::layout::Size {
    raw
});

impl<'gcx> HashStable<StableHashingContext<'gcx>> for LayoutError<'gcx>
{
    fn hash_stable<W: StableHasherResult>(&self,
                                          hcx: &mut StableHashingContext<'gcx>,
                                          hasher: &mut StableHasher<W>) {
        use ty::layout::LayoutError::*;
        mem::discriminant(self).hash_stable(hcx, hasher);

        match *self {
            Unknown(t) |
            SizeOverflow(t) => t.hash_stable(hcx, hasher)
        }
    }
}
