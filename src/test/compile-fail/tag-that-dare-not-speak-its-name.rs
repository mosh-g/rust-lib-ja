// error-pattern:mismatched types: expected `char` but found
// Issue #876

#[no_core];

extern mod core;

fn last<T>(v: ~[const &T]) -> core::Option<T> {
    fail;
}

fn main() {
    let y;
    let x : char = last(y);
}
