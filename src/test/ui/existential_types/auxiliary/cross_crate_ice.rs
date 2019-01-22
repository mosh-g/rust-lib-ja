// Crate that exports an existential type. Used for testing cross-crate.

#![feature(const_fn)]
#![crate_type="rlib"]

#![feature(existential_type)]

pub existential type Foo: std::fmt::Debug;

pub fn foo() -> Foo {
    5
}

