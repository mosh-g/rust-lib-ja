trait repeat<A> { fn get() -> A; }

impl<A:Copy> @A: repeat<A> {
    fn get() -> A { *self }
}

fn repeater<A:Copy>(v: @A) -> repeat<A> {
    // Note: owned kind is not necessary as A appears in the trait type
    v as repeat::<A> // No
}

fn main() {
    let x = &3;
    let y = repeater(@x);
    assert *x == *(y.get());
}