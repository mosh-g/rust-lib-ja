#![feature(custom_attribute, attr_literals)]
#![miri(memory_size=0)]

fn bar() {
    let x = 5;
    assert_eq!(x, 6);
}

fn main() { //~ ERROR tried to allocate 4 more bytes, but only 0 bytes are free of the 0 byte memory
    bar();
}
