// aux-build:issue-59764.rs
// compile-flags:--extern issue_59764
// edition:2018
// run-rustfix

#![allow(warnings)]

// This tests the suggestion to import macros from the root of a crate. This aims to capture
// the case where a user attempts to import a macro from the definition location instead of the
// root of the crate and the macro is annotated with `#![macro_export]`.

// Edge cases..

mod renamed_import {
    use issue_59764::foo::makro as baz;
    //~^ ERROR unresolved import `issue_59764::foo::makro` [E0432]
}

// Simple case..

use issue_59764::foo::makro;
//~^ ERROR unresolved import `issue_59764::foo::makro` [E0432]

makro!(bar);
//~^ ERROR cannot determine resolution for the macro `makro`

fn main() {
    bar();
    //~^ ERROR cannot find function `bar` in this scope [E0425]
}
