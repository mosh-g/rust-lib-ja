/*!
The kind traits

Rust types can be classified in vairous useful ways according to
intrinsic properties of the type. These classifications, often called
'kinds', are represented as traits.

They cannot be implemented by user code, but are instead implemented
by the compiler automatically for the types to which they apply.

The 4 kinds are

* Copy - types that may be copied without allocation. This includes
  scalar types and managed pointers, and exludes owned pointers. It
  also excludes types that implement `Drop`.

* Send - owned types and types containing owned types.  These types
  may be transferred across task boundaries.

* Const - types that are deeply immutable. Const types are used for
  freezable data structures.

* Owned - types that do not contain borrowed pointers. Note that this
  meaning of 'owned' conflicts with 'owned pointers'. The two notions
  of ownership are different.

`Copy` types include both implicitly copyable types that the compiler
will copy automatically and non-implicitly copyable types that require
the `copy` keyword to copy. Types that do not implement `Copy` may
instead implement `Clone`.

*/

#[lang="copy"]
pub trait Copy {
    // Empty.
}

#[lang="send"]
pub trait Send {
    // Empty.
}

#[lang="const"]
pub trait Const {
    // Empty.
}

#[lang="owned"]
pub trait Owned {
    // Empty.
}
