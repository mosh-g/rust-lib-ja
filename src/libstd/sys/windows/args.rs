// Copyright 2012-2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

#![allow(dead_code)] // runtime init functions not used during testing

use os::windows::prelude::*;
use sys::windows::os::current_exe;
use sys::c;
use ffi::OsString;
use fmt;
use vec;
use core::iter;
use slice;
use path::PathBuf;

pub unsafe fn init(_argc: isize, _argv: *const *const u8) { }

pub unsafe fn cleanup() { }

pub fn args() -> Args {
    unsafe {
        let lp_cmd_line = c::GetCommandLineW();
        let parsed_args_list = parse_lp_cmd_line(
            lp_cmd_line as *const u16,
            || current_exe().map(PathBuf::into_os_string).unwrap_or_else(|_| OsString::new()));

        Args { parsed_args_list: parsed_args_list }
    }
}

/// Implements the Windows command-line argument parsing algorithm, described at
/// <https://docs.microsoft.com/en-us/previous-versions//17w5ykft(v=vs.85)>.
///
/// Windows includes a function to do this in shell32.dll,
/// but linking with that DLL causes the process to be registered as a GUI application.
/// GUI applications add a bunch of overhead, even if no windows are drawn. See
/// <https://randomascii.wordpress.com/2018/12/03/a-not-called-function-can-cause-a-5x-slowdown/>.
unsafe fn parse_lp_cmd_line<F: Fn() -> OsString>(lp_cmd_line: *const u16, exe_name: F)
                                                 -> vec::IntoIter<OsString> {
    const BACKSLASH: u16 = '\\' as u16;
    const QUOTE: u16 = '"' as u16;
    const TAB: u16 = '\t' as u16;
    const SPACE: u16 = ' ' as u16;
    let mut in_quotes = false;
    let mut was_in_quotes = false;
    let mut backslash_count: usize = 0;
    let mut ret_val = Vec::new();
    let mut cur = Vec::new();
    if lp_cmd_line.is_null() || *lp_cmd_line == 0 {
        ret_val.push(exe_name());
        return ret_val.into_iter();
    }
    let mut i = 0;
    // The executable name at the beginning is special.
    match *lp_cmd_line {
        // The executable name ends at the next quote mark,
        // no matter what.
        QUOTE => {
            loop {
                i += 1;
                if *lp_cmd_line.offset(i) == 0 {
                    ret_val.push(OsString::from_wide(
                        slice::from_raw_parts(lp_cmd_line.offset(1), i as usize - 1)
                    ));
                    return ret_val.into_iter();
                }
                if *lp_cmd_line.offset(i) == QUOTE {
                    break;
                }
            }
            ret_val.push(OsString::from_wide(
                slice::from_raw_parts(lp_cmd_line.offset(1), i as usize - 1)
            ));
            i += 1;
        }
        // Implement quirk: when they say whitespace here,
        // they include the entire ASCII control plane:
        // "However, if lpCmdLine starts with any amount of whitespace, CommandLineToArgvW
        // will consider the first argument to be an empty string. Excess whitespace at the
        // end of lpCmdLine is ignored."
        0...SPACE => {
            ret_val.push(OsString::new());
            i += 1;
        },
        // The executable name ends at the next whitespace,
        // no matter what.
        _ => {
            loop {
                i += 1;
                if *lp_cmd_line.offset(i) == 0 {
                    ret_val.push(OsString::from_wide(
                        slice::from_raw_parts(lp_cmd_line, i as usize)
                    ));
                    return ret_val.into_iter();
                }
                if let 0...SPACE = *lp_cmd_line.offset(i) {
                    break;
                }
            }
            ret_val.push(OsString::from_wide(
                slice::from_raw_parts(lp_cmd_line, i as usize)
            ));
            i += 1;
        }
    }
    loop {
        let c = *lp_cmd_line.offset(i);
        match c {
            // backslash
            BACKSLASH => {
                backslash_count += 1;
                was_in_quotes = false;
            },
            QUOTE if backslash_count % 2 == 0 => {
                cur.extend(iter::repeat(b'\\' as u16).take(backslash_count / 2));
                backslash_count = 0;
                if was_in_quotes {
                    cur.push('"' as u16);
                    was_in_quotes = false;
                } else {
                    was_in_quotes = in_quotes;
                    in_quotes = !in_quotes;
                }
            }
            QUOTE if backslash_count % 2 != 0 => {
                cur.extend(iter::repeat(b'\\' as u16).take(backslash_count / 2));
                backslash_count = 0;
                was_in_quotes = false;
                cur.push(b'"' as u16);
            }
            SPACE | TAB if !in_quotes => {
                cur.extend(iter::repeat(b'\\' as u16).take(backslash_count));
                if !cur.is_empty() || was_in_quotes {
                    ret_val.push(OsString::from_wide(&cur[..]));
                    cur.truncate(0);
                }
                backslash_count = 0;
                was_in_quotes = false;
            }
            0x00 => {
                cur.extend(iter::repeat(b'\\' as u16).take(backslash_count));
                // include empty quoted strings at the end of the arguments list
                if !cur.is_empty() || was_in_quotes || in_quotes {
                    ret_val.push(OsString::from_wide(&cur[..]));
                }
                break;
            }
            _ => {
                cur.extend(iter::repeat(b'\\' as u16).take(backslash_count));
                backslash_count = 0;
                was_in_quotes = false;
                cur.push(c);
            }
        }
        i += 1;
    }
    ret_val.into_iter()
}

pub struct Args {
    parsed_args_list: vec::IntoIter<OsString>,
}

pub struct ArgsInnerDebug<'a> {
    args: &'a Args,
}

impl<'a> fmt::Debug for ArgsInnerDebug<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("[")?;
        let mut first = true;
        for i in self.args.parsed_args_list.clone() {
            if !first {
                f.write_str(", ")?;
            }
            first = false;

            fmt::Debug::fmt(&i, f)?;
        }
        f.write_str("]")?;
        Ok(())
    }
}

impl Args {
    pub fn inner_debug(&self) -> ArgsInnerDebug {
        ArgsInnerDebug {
            args: self
        }
    }
}

impl Iterator for Args {
    type Item = OsString;
    fn next(&mut self) -> Option<OsString> { self.parsed_args_list.next() }
    fn size_hint(&self) -> (usize, Option<usize>) { self.parsed_args_list.size_hint() }
}

impl DoubleEndedIterator for Args {
    fn next_back(&mut self) -> Option<OsString> { self.parsed_args_list.next_back() }
}

impl ExactSizeIterator for Args {
    fn len(&self) -> usize { self.parsed_args_list.len() }
}

#[cfg(test)]
mod tests {
    use sys::windows::args::*;
    use ffi::OsString;

    fn chk(string: &str, parts: &[&str]) {
        let mut wide: Vec<u16> = OsString::from(string).encode_wide().collect();
        wide.push(0);
        let parsed = unsafe {
            parse_lp_cmd_line(wide.as_ptr() as *const u16, || OsString::from("TEST.EXE"))
        };
        let expected: Vec<OsString> = parts.iter().map(|k| OsString::from(k)).collect();
        assert_eq!(parsed.as_slice(), expected.as_slice());
    }

    #[test]
    fn empty() {
        chk("", &["TEST.EXE"]);
        chk("\0", &["TEST.EXE"]);
    }

    #[test]
    fn single_words() {
        chk("EXE one_word", &["EXE", "one_word"]);
        chk("EXE a", &["EXE", "a"]);
        chk("EXE 😅", &["EXE", "😅"]);
        chk("EXE 😅🤦", &["EXE", "😅🤦"]);
    }

    #[test]
    fn official_examples() {
        chk(r#"EXE "abc" d e"#, &["EXE", "abc", "d", "e"]);
        chk(r#"EXE a\\\b d"e f"g h"#, &["EXE", r#"a\\\b"#, "de fg", "h"]);
        chk(r#"EXE a\\\"b c d"#, &["EXE", r#"a\"b"#, "c", "d"]);
        chk(r#"EXE a\\\\"b c" d e"#, &["EXE", r#"a\\b c"#, "d", "e"]);
    }

    #[test]
    fn whitespace_behavior() {
        chk(r#" test"#, &["", "test"]);
        chk(r#"  test"#, &["", "test"]);
        chk(r#" test test2"#, &["", "test", "test2"]);
        chk(r#" test  test2"#, &["", "test", "test2"]);
        chk(r#"test test2 "#, &["test", "test2"]);
        chk(r#"test  test2 "#, &["test", "test2"]);
        chk(r#"test "#, &["test"]);
    }

    #[test]
    fn genius_quotes() {
        chk(r#"EXE "" """#, &["EXE", "", ""]);
        chk(r#"EXE "" """"#, &["EXE", "", "\""]);
        chk(
            r#"EXE "this is """all""" in the same argument""#,
            &["EXE", "this is \"all\" in the same argument"]
        );
        chk(r#"EXE "a"""#, &["EXE", "a\""]);
        chk(r#"EXE "a"" a"#, &["EXE", "a\"", "a"]);
    }
}
