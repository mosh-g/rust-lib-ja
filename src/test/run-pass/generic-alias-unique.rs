

fn id<T: Copy Send>(t: T) -> T { return t; }

fn main() {
    let expected = ~100;
    let actual = id::<~int>(expected);
    log(debug, *actual);
    assert (*expected == *actual);
}
