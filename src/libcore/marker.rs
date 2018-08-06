// Copyright 2012-2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Primitive traits and types representing basic properties of types.
//!
//! Rust types can be classified in various useful ways according to
//! their intrinsic properties. These classifications are represented
//! as traits.

#![stable(feature = "rust1", since = "1.0.0")]

use cell::UnsafeCell;
use cmp;
use hash::Hash;
use hash::Hasher;

/// Types that can be transferred across thread boundaries.
///
/// This trait is automatically implemented when the compiler determines it's
/// appropriate.
///
/// An example of a non-`Send` type is the reference-counting pointer
/// [`rc::Rc`][`Rc`]. If two threads attempt to clone [`Rc`]s that point to the same
/// reference-counted value, they might try to update the reference count at the
/// same time, which is [undefined behavior][ub] because [`Rc`] doesn't use atomic
/// operations. Its cousin [`sync::Arc`][arc] does use atomic operations (incurring
/// some overhead) and thus is `Send`.
///
/// See [the Nomicon](../../nomicon/send-and-sync.html) for more details.
///
/// [`Rc`]: ../../std/rc/struct.Rc.html
/// [arc]: ../../std/sync/struct.Arc.html
/// [ub]: ../../reference/behavior-considered-undefined.html
#[stable(feature = "rust1", since = "1.0.0")]
#[rustc_on_unimplemented(
    message="`{Self}` cannot be sent between threads safely",
    label="`{Self}` cannot be sent between threads safely"
)]
pub unsafe auto trait Send {
    // empty.
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<T: ?Sized> !Send for *const T { }
#[stable(feature = "rust1", since = "1.0.0")]
impl<T: ?Sized> !Send for *mut T { }

/// Types with a constant size known at compile time.
///
/// All type parameters have an implicit bound of `Sized`. The special syntax
/// `?Sized` can be used to remove this bound if it's not appropriate.
///
/// ```
/// # #![allow(dead_code)]
/// struct Foo<T>(T);
/// struct Bar<T: ?Sized>(T);
///
/// // struct FooUse(Foo<[i32]>); // error: Sized is not implemented for [i32]
/// struct BarUse(Bar<[i32]>); // OK
/// ```
///
/// The one exception is the implicit `Self` type of a trait. A trait does not
/// have an implicit `Sized` bound as this is incompatible with [trait object]s
/// where, by definition, the trait needs to work with all possible implementors,
/// and thus could be any size.
///
/// Although Rust will let you bind `Sized` to a trait, you won't
/// be able to use it to form a trait object later:
///
/// ```
/// # #![allow(unused_variables)]
/// trait Foo { }
/// trait Bar: Sized { }
///
/// struct Impl;
/// impl Foo for Impl { }
/// impl Bar for Impl { }
///
/// let x: &Foo = &Impl;    // OK
/// // let y: &Bar = &Impl; // error: the trait `Bar` cannot
///                         // be made into an object
/// ```
///
/// [trait object]: ../../book/first-edition/trait-objects.html
#[stable(feature = "rust1", since = "1.0.0")]
#[lang = "sized"]
#[rustc_on_unimplemented(
    message="the size for values of type `{Self}` cannot be known at compilation time",
    label="doesn't have a size known at compile-time",
    note="to learn more, visit <https://doc.rust-lang.org/book/second-edition/\
          ch19-04-advanced-types.html#dynamically-sized-types-and-sized>",
)]
#[fundamental] // for Default, for example, which requires that `[T]: !Default` be evaluatable
pub trait Sized {
    // Empty.
}

/// Types that can be "unsized" to a dynamically-sized type.
///
/// For example, the sized array type `[i8; 2]` implements `Unsize<[i8]>` and
/// `Unsize<fmt::Debug>`.
///
/// All implementations of `Unsize` are provided automatically by the compiler.
///
/// `Unsize` is implemented for:
///
/// - `[T; N]` is `Unsize<[T]>`
/// - `T` is `Unsize<Trait>` when `T: Trait`
/// - `Foo<..., T, ...>` is `Unsize<Foo<..., U, ...>>` if:
///   - `T: Unsize<U>`
///   - Foo is a struct
///   - Only the last field of `Foo` has a type involving `T`
///   - `T` is not part of the type of any other fields
///   - `Bar<T>: Unsize<Bar<U>>`, if the last field of `Foo` has type `Bar<T>`
///
/// `Unsize` is used along with [`ops::CoerceUnsized`][coerceunsized] to allow
/// "user-defined" containers such as [`rc::Rc`][rc] to contain dynamically-sized
/// types. See the [DST coercion RFC][RFC982] and [the nomicon entry on coercion][nomicon-coerce]
/// for more details.
///
/// [coerceunsized]: ../ops/trait.CoerceUnsized.html
/// [rc]: ../../std/rc/struct.Rc.html
/// [RFC982]: https://github.com/rust-lang/rfcs/blob/master/text/0982-dst-coercion.md
/// [nomicon-coerce]: ../../nomicon/coercions.html
#[unstable(feature = "unsize", issue = "27732")]
#[lang = "unsize"]
pub trait Unsize<T: ?Sized> {
    // Empty.
}

/// Types whose values can be duplicated simply by copying bits.
///
/// By default, variable bindings have 'move semantics.' In other
/// words:
///
/// ```
/// #[derive(Debug)]
/// struct Foo;
///
/// let x = Foo;
///
/// let y = x;
///
/// // `x` has moved into `y`, and so cannot be used
///
/// // println!("{:?}", x); // error: use of moved value
/// ```
///
/// However, if a type implements `Copy`, it instead has 'copy semantics':
///
/// ```
/// // We can derive a `Copy` implementation. `Clone` is also required, as it's
/// // a supertrait of `Copy`.
/// #[derive(Debug, Copy, Clone)]
/// struct Foo;
///
/// let x = Foo;
///
/// let y = x;
///
/// // `y` is a copy of `x`
///
/// println!("{:?}", x); // A-OK!
/// ```
///
/// It's important to note that in these two examples, the only difference is whether you
/// are allowed to access `x` after the assignment. Under the hood, both a copy and a move
/// can result in bits being copied in memory, although this is sometimes optimized away.
///
/// ## How can I implement `Copy`?
///
/// There are two ways to implement `Copy` on your type. The simplest is to use `derive`:
///
/// ```
/// #[derive(Copy, Clone)]
/// struct MyStruct;
/// ```
///
/// You can also implement `Copy` and `Clone` manually:
///
/// ```
/// struct MyStruct;
///
/// impl Copy for MyStruct { }
///
/// impl Clone for MyStruct {
///     fn clone(&self) -> MyStruct {
///         *self
///     }
/// }
/// ```
///
/// There is a small difference between the two: the `derive` strategy will also place a `Copy`
/// bound on type parameters, which isn't always desired.
///
/// ## What's the difference between `Copy` and `Clone`?
///
/// Copies happen implicitly, for example as part of an assignment `y = x`. The behavior of
/// `Copy` is not overloadable; it is always a simple bit-wise copy.
///
/// Cloning is an explicit action, `x.clone()`. The implementation of [`Clone`] can
/// provide any type-specific behavior necessary to duplicate values safely. For example,
/// the implementation of [`Clone`] for [`String`] needs to copy the pointed-to string
/// buffer in the heap. A simple bitwise copy of [`String`] values would merely copy the
/// pointer, leading to a double free down the line. For this reason, [`String`] is [`Clone`]
/// but not `Copy`.
///
/// [`Clone`] is a supertrait of `Copy`, so everything which is `Copy` must also implement
/// [`Clone`]. If a type is `Copy` then its [`Clone`] implementation only needs to return `*self`
/// (see the example above).
///
/// ## When can my type be `Copy`?
///
/// A type can implement `Copy` if all of its components implement `Copy`. For example, this
/// struct can be `Copy`:
///
/// ```
/// # #[allow(dead_code)]
/// struct Point {
///    x: i32,
///    y: i32,
/// }
/// ```
///
/// A struct can be `Copy`, and [`i32`] is `Copy`, therefore `Point` is eligible to be `Copy`.
/// By contrast, consider
///
/// ```
/// # #![allow(dead_code)]
/// # struct Point;
/// struct PointList {
///     points: Vec<Point>,
/// }
/// ```
///
/// The struct `PointList` cannot implement `Copy`, because [`Vec<T>`] is not `Copy`. If we
/// attempt to derive a `Copy` implementation, we'll get an error:
///
/// ```text
/// the trait `Copy` may not be implemented for this type; field `points` does not implement `Copy`
/// ```
///
/// ## When *can't* my type be `Copy`?
///
/// Some types can't be copied safely. For example, copying `&mut T` would create an aliased
/// mutable reference. Copying [`String`] would duplicate responsibility for managing the
/// [`String`]'s buffer, leading to a double free.
///
/// Generalizing the latter case, any type implementing [`Drop`] can't be `Copy`, because it's
/// managing some resource besides its own [`size_of::<T>`] bytes.
///
/// If you try to implement `Copy` on a struct or enum containing non-`Copy` data, you will get
/// the error [E0204].
///
/// [E0204]: ../../error-index.html#E0204
///
/// ## When *should* my type be `Copy`?
///
/// Generally speaking, if your type _can_ implement `Copy`, it should. Keep in mind, though,
/// that implementing `Copy` is part of the public API of your type. If the type might become
/// non-`Copy` in the future, it could be prudent to omit the `Copy` implementation now, to
/// avoid a breaking API change.
///
/// ## Additional implementors
///
/// In addition to the [implementors listed below][impls],
/// the following types also implement `Copy`:
///
/// * Function item types (i.e. the distinct types defined for each function)
/// * Function pointer types (e.g. `fn() -> i32`)
/// * Array types, for all sizes, if the item type also implements `Copy` (e.g. `[i32; 123456]`)
/// * Tuple types, if each component also implements `Copy` (e.g. `()`, `(i32, bool)`)
/// * Closure types, if they capture no value from the environment
///   or if all such captured values implement `Copy` themselves.
///   Note that variables captured by shared reference always implement `Copy`
///   (even if the referent doesn't),
///   while variables captured by mutable reference never implement `Copy`.
///
/// [`Vec<T>`]: ../../std/vec/struct.Vec.html
/// [`String`]: ../../std/string/struct.String.html
/// [`Drop`]: ../../std/ops/trait.Drop.html
/// [`size_of::<T>`]: ../../std/mem/fn.size_of.html
/// [`Clone`]: ../clone/trait.Clone.html
/// [`String`]: ../../std/string/struct.String.html
/// [`i32`]: ../../std/primitive.i32.html
/// [impls]: #implementors
#[stable(feature = "rust1", since = "1.0.0")]
#[lang = "copy"]
pub trait Copy : Clone {
    // Empty.
}

/// Types for which it is safe to share references between threads.
///
/// This trait is automatically implemented when the compiler determines
/// it's appropriate.
///
/// The precise definition is: a type `T` is `Sync` if and only if `&T` is
/// [`Send`][send]. In other words, if there is no possibility of
/// [undefined behavior][ub] (including data races) when passing
/// `&T` references between threads.
///
/// As one would expect, primitive types like [`u8`][u8] and [`f64`][f64]
/// are all `Sync`, and so are simple aggregate types containing them,
/// like tuples, structs and enums. More examples of basic `Sync`
/// types include "immutable" types like `&T`, and those with simple
/// inherited mutability, such as [`Box<T>`][box], [`Vec<T>`][vec] and
/// most other collection types. (Generic parameters need to be `Sync`
/// for their container to be `Sync`.)
///
/// A somewhat surprising consequence of the definition is that `&mut T`
/// is `Sync` (if `T` is `Sync`) even though it seems like that might
/// provide unsynchronized mutation. The trick is that a mutable
/// reference behind a shared reference (that is, `& &mut T`)
/// becomes read-only, as if it were a `& &T`. Hence there is no risk
/// of a data race.
///
/// Types that are not `Sync` are those that have "interior
/// mutability" in a non-thread-safe form, such as [`cell::Cell`][cell]
/// and [`cell::RefCell`][refcell]. These types allow for mutation of
/// their contents even through an immutable, shared reference. For
/// example the `set` method on [`Cell<T>`][cell] takes `&self`, so it requires
/// only a shared reference [`&Cell<T>`][cell]. The method performs no
/// synchronization, thus [`Cell`][cell] cannot be `Sync`.
///
/// Another example of a non-`Sync` type is the reference-counting
/// pointer [`rc::Rc`][rc]. Given any reference [`&Rc<T>`][rc], you can clone
/// a new [`Rc<T>`][rc], modifying the reference counts in a non-atomic way.
///
/// For cases when one does need thread-safe interior mutability,
/// Rust provides [atomic data types], as well as explicit locking via
/// [`sync::Mutex`][mutex] and [`sync::RwLock`][rwlock]. These types
/// ensure that any mutation cannot cause data races, hence the types
/// are `Sync`. Likewise, [`sync::Arc`][arc] provides a thread-safe
/// analogue of [`Rc`][rc].
///
/// Any types with interior mutability must also use the
/// [`cell::UnsafeCell`][unsafecell] wrapper around the value(s) which
/// can be mutated through a shared reference. Failing to doing this is
/// [undefined behavior][ub]. For example, [`transmute`][transmute]-ing
/// from `&T` to `&mut T` is invalid.
///
/// See [the Nomicon](../../nomicon/send-and-sync.html) for more
/// details about `Sync`.
///
/// [send]: trait.Send.html
/// [u8]: ../../std/primitive.u8.html
/// [f64]: ../../std/primitive.f64.html
/// [box]: ../../std/boxed/struct.Box.html
/// [vec]: ../../std/vec/struct.Vec.html
/// [cell]: ../cell/struct.Cell.html
/// [refcell]: ../cell/struct.RefCell.html
/// [rc]: ../../std/rc/struct.Rc.html
/// [arc]: ../../std/sync/struct.Arc.html
/// [atomic data types]: ../sync/atomic/index.html
/// [mutex]: ../../std/sync/struct.Mutex.html
/// [rwlock]: ../../std/sync/struct.RwLock.html
/// [unsafecell]: ../cell/struct.UnsafeCell.html
/// [ub]: ../../reference/behavior-considered-undefined.html
/// [transmute]: ../../std/mem/fn.transmute.html
#[stable(feature = "rust1", since = "1.0.0")]
#[lang = "sync"]
#[rustc_on_unimplemented(
    message="`{Self}` cannot be shared between threads safely",
    label="`{Self}` cannot be shared between threads safely"
)]
pub unsafe auto trait Sync {
    // FIXME(estebank): once support to add notes in `rustc_on_unimplemented`
    // lands in beta, and it has been extended to check whether a closure is
    // anywhere in the requirement chain, extend it as such (#48534):
    // ```
    // on(
    //     closure,
    //     note="`{Self}` cannot be shared safely, consider marking the closure `move`"
    // ),
    // ```

    // Empty
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<T: ?Sized> !Sync for *const T { }
#[stable(feature = "rust1", since = "1.0.0")]
impl<T: ?Sized> !Sync for *mut T { }

macro_rules! impls{
    ($t: ident) => (
        #[stable(feature = "rust1", since = "1.0.0")]
        impl<T:?Sized> Hash for $t<T> {
            #[inline]
            fn hash<H: Hasher>(&self, _: &mut H) {
            }
        }

        #[stable(feature = "rust1", since = "1.0.0")]
        impl<T:?Sized> cmp::PartialEq for $t<T> {
            fn eq(&self, _other: &$t<T>) -> bool {
                true
            }
        }

        #[stable(feature = "rust1", since = "1.0.0")]
        impl<T:?Sized> cmp::Eq for $t<T> {
        }

        #[stable(feature = "rust1", since = "1.0.0")]
        impl<T:?Sized> cmp::PartialOrd for $t<T> {
            fn partial_cmp(&self, _other: &$t<T>) -> Option<cmp::Ordering> {
                Option::Some(cmp::Ordering::Equal)
            }
        }

        #[stable(feature = "rust1", since = "1.0.0")]
        impl<T:?Sized> cmp::Ord for $t<T> {
            fn cmp(&self, _other: &$t<T>) -> cmp::Ordering {
                cmp::Ordering::Equal
            }
        }

        #[stable(feature = "rust1", since = "1.0.0")]
        impl<T:?Sized> Copy for $t<T> { }

        #[stable(feature = "rust1", since = "1.0.0")]
        impl<T:?Sized> Clone for $t<T> {
            fn clone(&self) -> $t<T> {
                $t
            }
        }

        #[stable(feature = "rust1", since = "1.0.0")]
        impl<T:?Sized> Default for $t<T> {
            fn default() -> $t<T> {
                $t
            }
        }
        )
}

/// Zero-sized type used to mark things that "act like" they own a `T`.
///
/// Adding a `PhantomData<T>` field to your type tells the compiler that your
/// type acts as though it stores a value of type `T`, even though it doesn't
/// really. This information is used when computing certain safety properties.
///
/// For a more in-depth explanation of how to use `PhantomData<T>`, please see
/// [the Nomicon](../../nomicon/phantom-data.html).
///
/// # A ghastly note 👻👻👻
///
/// Though they both have scary names, `PhantomData` and 'phantom types' are
/// related, but not identical. A phantom type parameter is simply a type
/// parameter which is never used. In Rust, this often causes the compiler to
/// complain, and the solution is to add a "dummy" use by way of `PhantomData`.
///
/// # Examples
///
/// ## Unused lifetime parameters
///
/// Perhaps the most common use case for `PhantomData` is a struct that has an
/// unused lifetime parameter, typically as part of some unsafe code. For
/// example, here is a struct `Slice` that has two pointers of type `*const T`,
/// presumably pointing into an array somewhere:
///
/// ```compile_fail,E0392
/// struct Slice<'a, T> {
///     start: *const T,
///     end: *const T,
/// }
/// ```
///
/// The intention is that the underlying data is only valid for the
/// lifetime `'a`, so `Slice` should not outlive `'a`. However, this
/// intent is not expressed in the code, since there are no uses of
/// the lifetime `'a` and hence it is not clear what data it applies
/// to. We can correct this by telling the compiler to act *as if* the
/// `Slice` struct contained a reference `&'a T`:
///
/// ```
/// use std::marker::PhantomData;
///
/// # #[allow(dead_code)]
/// struct Slice<'a, T: 'a> {
///     start: *const T,
///     end: *const T,
///     phantom: PhantomData<&'a T>,
/// }
/// ```
///
/// This also in turn requires the annotation `T: 'a`, indicating
/// that any references in `T` are valid over the lifetime `'a`.
///
/// When initializing a `Slice` you simply provide the value
/// `PhantomData` for the field `phantom`:
///
/// ```
/// # #![allow(dead_code)]
/// # use std::marker::PhantomData;
/// # struct Slice<'a, T: 'a> {
/// #     start: *const T,
/// #     end: *const T,
/// #     phantom: PhantomData<&'a T>,
/// # }
/// fn borrow_vec<'a, T>(vec: &'a Vec<T>) -> Slice<'a, T> {
///     let ptr = vec.as_ptr();
///     Slice {
///         start: ptr,
///         end: unsafe { ptr.offset(vec.len() as isize) },
///         phantom: PhantomData,
///     }
/// }
/// ```
///
/// ## Unused type parameters
///
/// It sometimes happens that you have unused type parameters which
/// indicate what type of data a struct is "tied" to, even though that
/// data is not actually found in the struct itself. Here is an
/// example where this arises with [FFI]. The foreign interface uses
/// handles of type `*mut ()` to refer to Rust values of different
/// types. We track the Rust type using a phantom type parameter on
/// the struct `ExternalResource` which wraps a handle.
///
/// [FFI]: ../../book/first-edition/ffi.html
///
/// ```
/// # #![allow(dead_code)]
/// # trait ResType { }
/// # struct ParamType;
/// # mod foreign_lib {
/// #     pub fn new(_: usize) -> *mut () { 42 as *mut () }
/// #     pub fn do_stuff(_: *mut (), _: usize) {}
/// # }
/// # fn convert_params(_: ParamType) -> usize { 42 }
/// use std::marker::PhantomData;
/// use std::mem;
///
/// struct ExternalResource<R> {
///    resource_handle: *mut (),
///    resource_type: PhantomData<R>,
/// }
///
/// impl<R: ResType> ExternalResource<R> {
///     fn new() -> ExternalResource<R> {
///         let size_of_res = mem::size_of::<R>();
///         ExternalResource {
///             resource_handle: foreign_lib::new(size_of_res),
///             resource_type: PhantomData,
///         }
///     }
///
///     fn do_stuff(&self, param: ParamType) {
///         let foreign_params = convert_params(param);
///         foreign_lib::do_stuff(self.resource_handle, foreign_params);
///     }
/// }
/// ```
///
/// ## Ownership and the drop check
///
/// Adding a field of type `PhantomData<T>` indicates that your
/// type owns data of type `T`. This in turn implies that when your
/// type is dropped, it may drop one or more instances of the type
/// `T`. This has bearing on the Rust compiler's [drop check]
/// analysis.
///
/// If your struct does not in fact *own* the data of type `T`, it is
/// better to use a reference type, like `PhantomData<&'a T>`
/// (ideally) or `PhantomData<*const T>` (if no lifetime applies), so
/// as not to indicate ownership.
///
/// [drop check]: ../../nomicon/dropck.html
#[lang = "phantom_data"]
#[stable(feature = "rust1", since = "1.0.0")]
pub struct PhantomData<T:?Sized>;

impls! { PhantomData }

mod impls {
    #[stable(feature = "rust1", since = "1.0.0")]
    unsafe impl<'a, T: Sync + ?Sized> Send for &'a T {}
    #[stable(feature = "rust1", since = "1.0.0")]
    unsafe impl<'a, T: Send + ?Sized> Send for &'a mut T {}
}

/// Compiler-internal trait used to determine whether a type contains
/// any `UnsafeCell` internally, but not through an indirection.
/// This affects, for example, whether a `static` of that type is
/// placed in read-only static memory or writable static memory.
#[lang = "freeze"]
unsafe auto trait Freeze {}

impl<T: ?Sized> !Freeze for UnsafeCell<T> {}
unsafe impl<T: ?Sized> Freeze for PhantomData<T> {}
unsafe impl<T: ?Sized> Freeze for *const T {}
unsafe impl<T: ?Sized> Freeze for *mut T {}
unsafe impl<'a, T: ?Sized> Freeze for &'a T {}
unsafe impl<'a, T: ?Sized> Freeze for &'a mut T {}

/// Types that are safe to move.
///
/// Since moving objects is almost always safe, it is automatically implemented in most cases.
///
/// This trait is mainly used to build self referencial structs,
/// since moving an object with pointers to itself will invalidate them,
/// causing undefined behavior.
///
/// # The Pin API
///
/// The `Unpin` trait doesn't actually change the behavior of the compiler around moves,
/// so code like this will compile just fine:
///
/// ```rust
/// #![feature(pin)]
/// use std::marker::Pinned;
///
/// struct Unmovable {
///     _pin: Pinned, // this marker type prevents Unpin from being implemented for this type
/// }
///
/// let unmoved = Unmovable { _pin: Pinned };
/// let moved = unmoved;
/// ```
///
/// In order to actually prevent the pinned objects from moving,
/// it has to be wrapped in special pointer types,
/// which currently include [`PinMut`] and [`PinBox`].
///
/// The way they work is by implementing [`DerefMut`] for all types that implement Unpin,
/// but only [`Deref`] otherwise.
///
/// This is done because, while modifying an object can be done in-place,
/// it might also relocate a buffer when its at full capacity,
/// or it might replace one object with another without logically "moving" them with [`swap`].
///
/// [`PinMut`]: ../mem/struct.PinMut.html
/// [`PinBox`]: ../../alloc/boxed/struct.PinBox.html
/// [`DerefMut`]: ../ops/trait.DerefMut.html
/// [`Deref`]: ../ops/trait.Deref.html
/// [`swap`]: ../mem/fn.swap.html
///
/// # Examples
///
/// ```rust
/// #![feature(pin)]
///
/// use std::boxed::PinBox;
/// use std::marker::Pinned;
/// use std::ptr::NonNull;
///
/// // This is a self referencial struct since the slice field points to the data field.
/// // We cannot inform the compiler about that with a normal reference,
/// // since this pattern cannot be described with the usual borrowing rules.
/// // Instead we use a raw pointer, though one which is known to not be null,
/// // since we know it's pointing at the string.
/// struct Unmovable {
///     data: String,
///     slice: NonNull<String>,
///     _pin: Pinned,
/// }
///
/// impl Unmovable {
///     // To ensure the data doesn't move when the function returns,
///     // we place it in the heap where it will stay for the lifetime of the object,
///     // and the only way to access it would be through a pointer to it.
///     fn new(data: String) -> PinBox<Self> {
///         let res = Unmovable {
///             data,
///             // we only create the pointer once the data is in place
///             // otherwise it will have already moved before we even started
///             slice: NonNull::dangling(),
///             _pin: Pinned,
///         };
///         let mut boxed = PinBox::new(res);
///
///         let slice = NonNull::from(&boxed.data);
///         // we know this is safe because modifying a field doesn't move the whole struct
///         unsafe { PinBox::get_mut(&mut boxed).slice = slice };
///         boxed
///     }
/// }
///
/// let unmoved = Unmovable::new("hello".to_string());
/// // The pointer should point to the correct location,
/// // so long as the struct hasn't moved.
/// // Meanwhile, we are free to move the pointer around.
/// let mut still_unmoved = unmoved;
/// assert_eq!(still_unmoved.slice, NonNull::from(&still_unmoved.data));
///
/// // Now the only way to access to data (safely) is immutably,
/// // so this will fail to compile:
/// // still_unmoved.data.push_str(" world");
///
/// ```
#[unstable(feature = "pin", issue = "49150")]
pub auto trait Unpin {}

/// A type which does not implement `Unpin`.
///
/// If a type contains a `Pinned`, it will not implement `Unpin` by default.
#[unstable(feature = "pin", issue = "49150")]
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Pinned;

#[unstable(feature = "pin", issue = "49150")]
impl !Unpin for Pinned {}

#[unstable(feature = "pin", issue = "49150")]
impl<'a, T: ?Sized + 'a> Unpin for &'a T {}

#[unstable(feature = "pin", issue = "49150")]
impl<'a, T: ?Sized + 'a> Unpin for &'a mut T {}

/// Implementations of `Copy` for primitive types.
///
/// Implementations that cannot be described in Rust
/// are implemented in `SelectionContext::copy_clone_conditions()` in librustc.
mod copy_impls {

    use super::Copy;

    macro_rules! impl_copy {
        ($($t:ty)*) => {
            $(
                #[stable(feature = "rust1", since = "1.0.0")]
                impl Copy for $t {}
            )*
        }
    }

    impl_copy! {
        usize u8 u16 u32 u64 u128
        isize i8 i16 i32 i64 i128
        f32 f64
        bool char
    }

    #[unstable(feature = "never_type", issue = "35121")]
    impl Copy for ! {}

    #[stable(feature = "rust1", since = "1.0.0")]
    impl<T: ?Sized> Copy for *const T {}

    #[stable(feature = "rust1", since = "1.0.0")]
    impl<T: ?Sized> Copy for *mut T {}

    // Shared references can be copied, but mutable references *cannot*!
    #[stable(feature = "rust1", since = "1.0.0")]
    impl<'a, T: ?Sized> Copy for &'a T {}

}
