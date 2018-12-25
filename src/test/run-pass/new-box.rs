#![feature(box_syntax)]

fn f(x: Box<isize>) {
    let y: &isize = &*x;
    println!("{}", *x);
    println!("{}", *y);
}

trait Trait {
    fn printme(&self);
}

struct Struct;

impl Trait for Struct {
    fn printme(&self) {
        println!("hello world!");
    }
}

fn g(x: Box<Trait>) {
    x.printme();
    let y: &Trait = &*x;
    y.printme();
}

fn main() {
    f(box 1234);
    g(box Struct as Box<Trait>);
}
