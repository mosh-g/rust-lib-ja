fn main() {
    /// hi
    //~^ ERROR found a documentation comment that doesn't document anything
    //~| HELP maybe a comment was intended
    ;
    //~^ WARNING unnecessary trailing semicolon
    //~| HELP remove this semicolon
}
