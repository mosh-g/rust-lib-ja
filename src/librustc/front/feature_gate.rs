// Copyright 2013 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Feature gating
//!
//! This modules implements the gating necessary for preventing certain compiler
//! features from being used by default. This module will crawl a pre-expanded
//! AST to ensure that there are no features which are used that are not
//! enabled.
//!
//! Features are enabled in programs via the crate-level attributes of
//! #[feature(...)] with a comma-separated list of features.

use syntax::ast;
use syntax::attr::AttrMetaMethods;
use syntax::codemap::Span;
use syntax::visit;
use syntax::visit::Visitor;

use driver::session::Session;

/// This is a list of all known features since the beginning of time. This list
/// can never shrink, it may only be expanded (in order to prevent old programs
/// from failing to compile). The status of each feature may change, however.
static KNOWN_FEATURES: &'static [(&'static str, Status)] = &[
    ("globs", Active),
    ("macro_rules", Active),
    ("struct_variant", Active),

    // These are used to test this portion of the compiler, they don't actually
    // mean anything
    ("test_accepted_feature", Accepted),
    ("test_removed_feature", Removed),
];

enum Status {
    /// Represents an active feature that is currently being implemented or
    /// currently being considered for addition/removal.
    Active,

    /// Represents a feature which has since been removed (it was once Active)
    Removed,

    /// This language feature has since been Accepted (it was once Active)
    Accepted,
}

struct Context {
    features: ~[&'static str],
    sess: Session,
}

impl Context {
    fn gate_feature(&self, feature: &str, span: Span, explain: &str) {
        if !self.has_feature(feature) {
            self.sess.span_err(span, explain);
            self.sess.span_note(span, format!("add \\#[feature({})] to the \
                                                  crate attributes to enable",
                                                 feature));
        }
    }

    fn has_feature(&self, feature: &str) -> bool {
        self.features.iter().any(|n| n.as_slice() == feature)
    }
}

impl Visitor<()> for Context {
    fn visit_view_item(&mut self, i: &ast::view_item, _: ()) {
        match i.node {
            ast::view_item_use(ref paths) => {
                for path in paths.iter() {
                    match path.node {
                        ast::view_path_glob(*) => {
                            self.gate_feature("globs", path.span,
                                              "glob import statements are \
                                               experimental and possibly buggy");
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        visit::walk_view_item(self, i, ())
    }

    fn visit_item(&mut self, i: @ast::item, _:()) {
        match i.node {
            ast::item_enum(ref def, _) => {
                for variant in def.variants.iter() {
                    match variant.node.kind {
                        ast::struct_variant_kind(*) => {
                            self.gate_feature("struct_variant", variant.span,
                                              "enum struct variants are \
                                               experimental and possibly buggy");
                        }
                        _ => {}
                    }
                }
            }

            ast::item_mac(ref mac) => {
                match mac.node {
                    ast::mac_invoc_tt(ref path, _, _) => {
                        let rules = self.sess.ident_of("macro_rules");
                        if path.segments.last().identifier == rules {
                            self.gate_feature("macro_rules", i.span,
                                              "macro definitions are not \
                                               stable enough for use and are \
                                               subject to change");
                        }
                    }
                }
            }

            _ => {}
        }

        visit::walk_item(self, i, ());
    }
}

pub fn check_crate(sess: Session, crate: &ast::Crate) {
    let mut cx = Context {
        features: ~[],
        sess: sess,
    };

    for attr in crate.attrs.iter() {
        if "feature" != attr.name() { continue }

        match attr.meta_item_list() {
            None => {
                sess.span_err(attr.span, "malformed feature attribute, \
                                          expected #[feature(...)]");
            }
            Some(list) => {
                for &mi in list.iter() {
                    let name = match mi.node {
                        ast::MetaWord(word) => word,
                        _ => {
                            sess.span_err(mi.span, "malformed feature, expected \
                                                    just one word");
                            continue
                        }
                    };
                    match KNOWN_FEATURES.iter().find(|& &(n, _)| n == name) {
                        Some(&(name, Active)) => { cx.features.push(name); }
                        Some(&(_, Removed)) => {
                            sess.span_err(mi.span, "feature has been removed");
                        }
                        Some(&(_, Accepted)) => {
                            sess.span_warn(mi.span, "feature has added to rust, \
                                                     directive not necessary");
                        }
                        None => {
                            sess.span_err(mi.span, "unknown feature");
                        }
                    }
                }
            }
        }
    }

    visit::walk_crate(&mut cx, crate, ());

    sess.abort_if_errors();
}
