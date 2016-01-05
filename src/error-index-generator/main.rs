// Copyright 2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

#![feature(rustc_private, rustdoc)]

extern crate syntax;
extern crate rustdoc;
extern crate serialize as rustc_serialize;

use std::collections::BTreeMap;
use std::fs::{read_dir, File};
use std::io::{Read, Write};
use std::env;
use std::path::Path;
use std::error::Error;

use syntax::diagnostics::metadata::{get_metadata_dir, ErrorMetadataMap, ErrorMetadata};

use rustdoc::html::markdown::Markdown;
use rustc_serialize::json;

enum OutputFormat {
    HTML(HTMLFormatter),
    Markdown(MarkdownFormatter),
    Unknown(String),
}

impl OutputFormat {
    fn from(format: &str) -> OutputFormat {
        match &*format.to_lowercase() {
            "html"     => OutputFormat::HTML(HTMLFormatter),
            "markdown" => OutputFormat::Markdown(MarkdownFormatter),
            s          => OutputFormat::Unknown(s.to_owned()),
        }
    }
}

trait Formatter {
    fn header(&self, output: &mut Write) -> Result<(), Box<Error>>;
    fn title(&self, output: &mut Write) -> Result<(), Box<Error>>;
    fn error_code_block(&self, output: &mut Write, info: &ErrorMetadata,
                        err_code: &str) -> Result<(), Box<Error>>;
    fn footer(&self, output: &mut Write) -> Result<(), Box<Error>>;
}

struct HTMLFormatter;
struct MarkdownFormatter;

impl Formatter for HTMLFormatter {
    fn header(&self, output: &mut Write) -> Result<(), Box<Error>> {
        try!(write!(output, r##"<!DOCTYPE html>
<html>
<head>
<title>Rust Compiler Error Index</title>
<meta charset="utf-8">
<!-- Include rust.css after main.css so its rules take priority. -->
<link rel="stylesheet" type="text/css" href="main.css"/>
<link rel="stylesheet" type="text/css" href="rust.css"/>
<style>
.error-undescribed {{
    display: none;
}}
</style>
</head>
<body>
"##));
        Ok(())
    }

    fn title(&self, output: &mut Write) -> Result<(), Box<Error>> {
        try!(write!(output, "<h1>Rust Compiler Error Index</h1>\n"));
        Ok(())
    }

    fn error_code_block(&self, output: &mut Write, info: &ErrorMetadata,
                        err_code: &str) -> Result<(), Box<Error>> {
        // Enclose each error in a div so they can be shown/hidden en masse.
        let desc_desc = match info.description {
            Some(_) => "error-described",
            None => "error-undescribed",
        };
        let use_desc = match info.use_site {
            Some(_) => "error-used",
            None => "error-unused",
        };
        try!(write!(output, "<div class=\"{} {}\">", desc_desc, use_desc));

        // Error title (with self-link).
        try!(write!(output,
                    "<h2 id=\"{0}\" class=\"section-header\"><a href=\"#{0}\">{0}</a></h2>\n",
                    err_code));

        // Description rendered as markdown.
        match info.description {
            Some(ref desc) => try!(write!(output, "{}", Markdown(desc))),
            None => try!(write!(output, "<p>No description.</p>\n")),
        }

        try!(write!(output, "</div>\n"));
        Ok(())
    }

    fn footer(&self, output: &mut Write) -> Result<(), Box<Error>> {
        try!(write!(output, "</body>\n</html>"));
        Ok(())
    }
}

impl Formatter for MarkdownFormatter {
    #[allow(unused_variables)]
    fn header(&self, output: &mut Write) -> Result<(), Box<Error>> {
        Ok(())
    }

    fn title(&self, output: &mut Write) -> Result<(), Box<Error>> {
        try!(write!(output, "# Rust Compiler Error Index\n"));
        Ok(())
    }

    fn error_code_block(&self, output: &mut Write, info: &ErrorMetadata,
                        err_code: &str) -> Result<(), Box<Error>> {
        Ok(match info.description {
            Some(ref desc) => try!(write!(output, "## {}\n{}\n", err_code, desc)),
            None => (),
        })
    }

    #[allow(unused_variables)]
    fn footer(&self, output: &mut Write) -> Result<(), Box<Error>> {
        Ok(())
    }
}

/// Load all the metadata files from `metadata_dir` into an in-memory map.
fn load_all_errors(metadata_dir: &Path) -> Result<ErrorMetadataMap, Box<Error>> {
    let mut all_errors = BTreeMap::new();

    for entry in try!(read_dir(metadata_dir)) {
        let path = try!(entry).path();

        let mut metadata_str = String::new();
        try!(File::open(&path).and_then(|mut f| f.read_to_string(&mut metadata_str)));

        let some_errors: ErrorMetadataMap = try!(json::decode(&metadata_str));

        for (err_code, info) in some_errors {
            all_errors.insert(err_code, info);
        }
    }

    Ok(all_errors)
}

/// Output an HTML page for the errors in `err_map` to `output_path`.
fn render_error_page<T: Formatter>(err_map: &ErrorMetadataMap, output_path: &Path,
                                   formatter: T) -> Result<(), Box<Error>> {
    let mut output_file = try!(File::create(output_path));

    try!(formatter.header(&mut output_file));
    try!(formatter.title(&mut output_file));

    for (err_code, info) in err_map {
        try!(formatter.error_code_block(&mut output_file, info, err_code));
    }

    formatter.footer(&mut output_file)
}

fn main_with_result(format: OutputFormat) -> Result<(), Box<Error>> {
    let build_arch = try!(env::var("CFG_BUILD"));
    let metadata_dir = get_metadata_dir(&build_arch);
    let err_map = try!(load_all_errors(&metadata_dir));
    match format {
        OutputFormat::Unknown(s)    => panic!("Unknown output format: {}", s),
        OutputFormat::HTML(h)       => try!(render_error_page(&err_map,
                                                              Path::new("doc/error-index.html"),
                                                              h)),
        OutputFormat::Markdown(m)   => try!(render_error_page(&err_map,
                                                              Path::new("doc/error-index.html"),
                                                              m)),
    }
    Ok(())
}

fn parse_args() -> OutputFormat {
    for arg in env::args().skip(1) {
        return OutputFormat::from(&arg);
    }
    OutputFormat::from("html")
}

fn main() {
    if let Err(e) = main_with_result(parse_args()) {
        panic!("{}", e.description());
    }
}
