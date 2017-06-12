// Copyright 2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Tidy check to ensure that unstable features are all in order
//!
//! This check will ensure properties like:
//!
//! * All stability attributes look reasonably well formed
//! * The set of library features is disjoint from the set of language features
//! * Library features have at most one stability level
//! * Library features have at most one `since` value
//! * All unstable lang features have tests to ensure they are actually unstable

use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::prelude::*;
use std::path::Path;

#[derive(Debug, PartialEq, Clone)]
pub enum Status {
    Stable,
    Removed,
    Unstable,
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let as_str = match *self {
            Status::Stable => "stable",
            Status::Unstable => "unstable",
            Status::Removed => "removed",
        };
        fmt::Display::fmt(as_str, f)
    }
}

#[derive(Debug, Clone)]
pub struct Feature {
    pub level: Status,
    pub since: String,
    pub has_gate_test: bool,
    pub tracking_issue: Option<u32>,
}

pub type Features = HashMap<String, Feature>;

pub fn check(path: &Path, bad: &mut bool, quiet: bool) {
    let mut features = collect_lang_features(path);
    assert!(!features.is_empty());

    let lib_features = collect_lib_features(path, bad, &features);
    assert!(!lib_features.is_empty());

    let mut contents = String::new();

    super::walk_many(&[&path.join("test/compile-fail"),
                       &path.join("test/compile-fail-fulldeps"),
                       &path.join("test/parse-fail"),],
                     &mut |path| super::filter_dirs(path),
                     &mut |file| {
        let filename = file.file_name().unwrap().to_string_lossy();
        if !filename.ends_with(".rs") || filename == "features.rs" ||
           filename == "diagnostic_list.rs" {
            return;
        }

        let filen_underscore = filename.replace("-","_").replace(".rs","");
        let filename_is_gate_test = test_filen_gate(&filen_underscore, &mut features);

        contents.truncate(0);
        t!(t!(File::open(&file), &file).read_to_string(&mut contents));

        for (i, line) in contents.lines().enumerate() {
            let mut err = |msg: &str| {
                tidy_error!(bad, "{}:{}: {}", file.display(), i + 1, msg);
            };

            let gate_test_str = "gate-test-";

            if !line.contains(gate_test_str) {
                continue;
            }

            let feature_name = match line.find(gate_test_str) {
                Some(i) => {
                    &line[i+gate_test_str.len()..line[i+1..].find(' ').unwrap_or(line.len())]
                },
                None => continue,
            };
            match features.get_mut(feature_name) {
                Some(f) => {
                    if filename_is_gate_test {
                        err(&format!("The file is already marked as gate test \
                                      through its name, no need for a \
                                      'gate-test-{}' comment",
                                     feature_name));
                    }
                    f.has_gate_test = true;
                }
                None => {
                    err(&format!("gate-test test found referencing a nonexistent feature '{}'",
                                 feature_name));
                }
            }
        }
    });

    // Only check the number of lang features.
    // Obligatory testing for library features is dumb.
    let gate_untested = features.iter()
                                .filter(|&(_, f)| f.level == Status::Unstable)
                                .filter(|&(_, f)| !f.has_gate_test)
                                .collect::<Vec<_>>();

    for &(name, _) in gate_untested.iter() {
        println!("Expected a gate test for the feature '{}'.", name);
        println!("Hint: create a file named 'feature-gate-{}.rs' in the compile-fail\
                \n      test suite, with its failures due to missing usage of\
                \n      #![feature({})].", name, name);
        println!("Hint: If you already have such a test and don't want to rename it,\
                \n      you can also add a // gate-test-{} line to the test file.",
                 name);
    }

    if gate_untested.len() > 0 {
        tidy_error!(bad, "Found {} features without a gate test.", gate_untested.len());
    }

    if *bad {
        return;
    }
    if quiet {
        println!("* {} features", features.len());
        return;
    }

    let mut lines = Vec::new();
    for (name, feature) in features.iter() {
        lines.push(format!("{:<32} {:<8} {:<12} {:<8}",
                           name,
                           "lang",
                           feature.level,
                           feature.since));
    }
    for (name, feature) in lib_features {
        lines.push(format!("{:<32} {:<8} {:<12} {:<8}",
                           name,
                           "lib",
                           feature.level,
                           feature.since));
    }

    lines.sort();
    for line in lines {
        println!("* {}", line);
    }
}

fn find_attr_val<'a>(line: &'a str, attr: &str) -> Option<&'a str> {
    line.find(attr)
        .and_then(|i| line[i..].find('"').map(|j| i + j + 1))
        .and_then(|i| line[i..].find('"').map(|j| (i, i + j)))
        .map(|(i, j)| &line[i..j])
}

fn test_filen_gate(filen_underscore: &str, features: &mut Features) -> bool {
    if filen_underscore.starts_with("feature_gate") {
        for (n, f) in features.iter_mut() {
            if filen_underscore == format!("feature_gate_{}", n) {
                f.has_gate_test = true;
                return true;
            }
        }
    }
    return false;
}

pub fn collect_lang_features(base_src_path: &Path) -> Features {
    let mut contents = String::new();
    let path = base_src_path.join("libsyntax/feature_gate.rs");
    t!(t!(File::open(path)).read_to_string(&mut contents));

    contents.lines()
        .filter_map(|line| {
            let mut parts = line.trim().split(",");
            let level = match parts.next().map(|l| l.trim().trim_left_matches('(')) {
                Some("active") => Status::Unstable,
                Some("removed") => Status::Removed,
                Some("accepted") => Status::Stable,
                _ => return None,
            };
            let name = parts.next().unwrap().trim();
            let since = parts.next().unwrap().trim().trim_matches('"');
            let issue_str = parts.next().unwrap().trim();
            let tracking_issue = if issue_str.starts_with("None") {
                None
            } else {
                let s = issue_str.split("(").nth(1).unwrap().split(")").nth(0).unwrap();
                Some(s.parse().unwrap())
            };
            Some((name.to_owned(),
                Feature {
                    level,
                    since: since.to_owned(),
                    has_gate_test: false,
                    tracking_issue,
                }))
        })
        .collect()
}

pub fn collect_lib_features(base_src_path: &Path,
                            bad: &mut bool,
                            features: &Features) -> Features {
    let mut lib_features = Features::new();
    let mut contents = String::new();
    super::walk(base_src_path,
                &mut |path| super::filter_dirs(path) || path.ends_with("src/test"),
                &mut |file| {
        let filename = file.file_name().unwrap().to_string_lossy();
        if !filename.ends_with(".rs") || filename == "features.rs" ||
           filename == "diagnostic_list.rs" {
            return;
        }

        contents.truncate(0);
        t!(t!(File::open(&file), &file).read_to_string(&mut contents));

        let mut becoming_feature: Option<(String, Feature)> = None;
        for (i, line) in contents.lines().enumerate() {
            let mut err = |msg: &str| {
                tidy_error!(bad, "{}:{}: {}", file.display(), i + 1, msg);
            };
            if let Some((ref name, ref mut f)) = becoming_feature {
                if f.tracking_issue.is_none() {
                    f.tracking_issue = find_attr_val(line, "issue")
                    .map(|s| s.parse().unwrap());
                }
                if line.ends_with("]") {
                    lib_features.insert(name.to_owned(), f.clone());
                } else if !line.ends_with(",") && !line.ends_with("\\") {
                    // We need to bail here because we might have missed the
                    // end of a stability attribute above because the "]"
                    // might not have been at the end of the line.
                    // We could then get into the very unfortunate situation that
                    // we continue parsing the file assuming the current stability
                    // attribute has not ended, and ignoring possible feature
                    // attributes in the process.
                    err("malformed stability attribute");
                } else {
                    continue;
                }
            }
            becoming_feature = None;
            let level = if line.contains("[unstable(") {
                Status::Unstable
            } else if line.contains("[stable(") {
                Status::Stable
            } else {
                continue;
            };
            let feature_name = match find_attr_val(line, "feature") {
                Some(name) => name,
                None => {
                    err("malformed stability attribute");
                    continue;
                }
            };
            let since = match find_attr_val(line, "since") {
                Some(name) => name,
                None if level == Status::Stable => {
                    err("malformed stability attribute");
                    continue;
                }
                None => "None",
            };
            let tracking_issue = find_attr_val(line, "issue").map(|s| s.parse().unwrap());

            if features.contains_key(feature_name) {
                err("duplicating a lang feature");
            }
            if let Some(ref s) = lib_features.get(feature_name) {
                if s.level != level {
                    err("different stability level than before");
                }
                if s.since != since {
                    err("different `since` than before");
                }
                continue;
            }
            let feature = Feature {
                level,
                since: since.to_owned(),
                has_gate_test: false,
                tracking_issue,
            };
            if line.contains("]") {
                lib_features.insert(feature_name.to_owned(), feature);
            } else {
                becoming_feature = Some((feature_name.to_owned(), feature));
            }
        }
    });
    lib_features
}
