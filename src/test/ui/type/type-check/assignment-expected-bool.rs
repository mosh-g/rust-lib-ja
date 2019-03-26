// The purpose of this text is to ensure that we get good
// diagnostics when a `bool` is expected but that due to
// an assignment expression `x = y` the type is `()`.

fn main() {
    let _: bool = 0 = 0; //~ ERROR mismatched types [E0308]

    let _: bool = match 0 {
        0 => 0 = 0, //~ ERROR mismatched types [E0308]
        _ => 0 = 0, //~ ERROR mismatched types [E0308]
    };

    let _: bool = match true {
        true => 0 = 0, //~ ERROR mismatched types [E0308]
        _ => (),
    };

    if 0 = 0 {} //~ ERROR mismatched types [E0308]

    let _: bool = if { 0 = 0 } { //~ ERROR mismatched types [E0308]
        0 = 0 //~ ERROR mismatched types [E0308]
    } else {
        0 = 0 //~ ERROR mismatched types [E0308]
    };

    let _ = (0 = 0) //~ ERROR mismatched types [E0308]
        && { 0 = 0 } //~ ERROR mismatched types [E0308]
        || (0 = 0); //~ ERROR mismatched types [E0308]
}
