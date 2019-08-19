// ignore-tidy-linelength
// ignore-compare-mode-nll
#![feature(const_in_array_repeat_expressions, nll)]
#![allow(warnings)]

// Some type that is not copyable.
struct Bar;

mod non_constants {
    use Bar;

    fn no_impl_copy_empty_value_multiple_elements() {
        let x = None;
        let arr: [Option<Bar>; 2] = [x; 2];
        //~^ ERROR the trait bound `std::option::Option<Bar>: std::marker::Copy` is not satisfied [E0277]
    }

    fn no_impl_copy_value_multiple_elements() {
        let x = Some(Bar);
        let arr: [Option<Bar>; 2] = [x; 2];
        //~^ ERROR the trait bound `std::option::Option<Bar>: std::marker::Copy` is not satisfied [E0277]
    }
}

fn main() {}
