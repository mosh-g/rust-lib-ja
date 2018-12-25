// More checks that stability attributes are used correctly

#![feature(staged_api)]

#![stable(feature = "stable_test_feature", since = "1.0.0")]

#[macro_export]
macro_rules! mac { //~ ERROR This node does not have a stability attribute
    () => ()
}

fn main() { }
