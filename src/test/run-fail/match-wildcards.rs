// error-pattern:squirrelcupcake
fn cmp() -> isize {
    match (Some('a'), None::<char>) {
        (Some(_), _) => {
            panic!("squirrelcupcake");
        }
        (_, Some(_)) => {
            panic!();
        }
        _ => {
            panic!("wat");
        }
    }
}

fn main() {
    println!("{}", cmp());
}
