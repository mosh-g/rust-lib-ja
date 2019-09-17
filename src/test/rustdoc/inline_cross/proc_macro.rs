// aux-build:proc_macro.rs
// build-aux-docs

extern crate some_macros;

// @has proc_macro/index.html
// @has - '//a/@href' 'macro.some_proc_macro.html'
// @has - '//a/@href' 'attr.some_proc_attr.html'
// @has - '//a/@href' 'derive.SomeDerive.html'
// @has proc_macro/macro.some_proc_macro.html
// @has proc_macro/attr.some_proc_attr.html
// @has proc_macro/derive.SomeDerive.html

// @has proc_macro/macro.some_proc_macro.html
// @has - 'a proc-macro that swallows its input and does nothing.'
pub use some_macros::some_proc_macro;

// @has proc_macro/macro.reexported_macro.html
// @has - 'Doc comment from the original crate'
pub use some_macros::reexported_macro;

// @has proc_macro/attr.some_proc_attr.html
// @has - 'a proc-macro attribute that passes its item through verbatim.'
pub use some_macros::some_proc_attr;

// @has proc_macro/derive.SomeDerive.html
// @has - 'a derive attribute that adds nothing to its input.'
pub use some_macros::SomeDerive;
