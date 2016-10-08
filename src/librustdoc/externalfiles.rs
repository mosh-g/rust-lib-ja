// Copyright 2014 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use std::fs::File;
use std::io::prelude::*;
use std::io;
use std::path::Path;
use std::str;

#[derive(Clone)]
pub struct ExternalHtml{
    /// Content that will be included inline in the <head> section of a
    /// rendered Markdown file or generated documentation
    pub in_header: String,
    /// Content that will be included inline between <body> and the content of
    /// a rendered Markdown file or generated documentation
    pub before_content: String,
    /// Content that will be included inline between the content and </body> of
    /// a rendered Markdown file or generated documentation
    pub after_content: String
}

impl ExternalHtml {
    pub fn load(in_header: &[String], before_content: &[String], after_content: &[String])
            -> Option<ExternalHtml> {
        match (load_external_files(in_header),
               load_external_files(before_content),
               load_external_files(after_content)) {
            (Some(ih), Some(bc), Some(ac)) => Some(ExternalHtml {
                in_header: ih,
                before_content: bc,
                after_content: ac
            }),
            _ => None
        }
    }
}

pub fn load_string(input: &Path) -> io::Result<Option<String>> {
    let mut f = File::open(input)?;
    let mut d = Vec::new();
    f.read_to_end(&mut d)?;
    Ok(str::from_utf8(&d).map(|s| s.to_string()).ok())
}

macro_rules! load_or_return {
    ($input: expr, $cant_read: expr, $not_utf8: expr) => {
        {
            let input = Path::new(&$input[..]);
            match ::externalfiles::load_string(input) {
                Err(e) => {
                    let _ = writeln!(&mut io::stderr(),
                                     "error reading `{}`: {}", input.display(), e);
                    return $cant_read;
                }
                Ok(None) => {
                    let _ = writeln!(&mut io::stderr(),
                                     "error reading `{}`: not UTF-8", input.display());
                    return $not_utf8;
                }
                Ok(Some(s)) => s
            }
        }
    }
}

pub fn load_external_files(names: &[String]) -> Option<String> {
    let mut out = String::new();
    for name in names {
        out.push_str(&*load_or_return!(&name, None, None));
        out.push('\n');
    }
    Some(out)
}
