#![feature(type_alias_impl_trait)]

use std::fmt::Debug;

fn main() {}

// test that unused generic parameters are ok
type Two<T, U> = impl Debug;
//~^ could not find defining uses

fn one<T: Debug>(t: T) -> Two<T, T> {
//~^ ERROR defining existential type use restricts existential type
    t
}
