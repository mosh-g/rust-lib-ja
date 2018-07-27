// Copyright 2012-2014 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use rustc::hir::def_id::DefId;
use rustc::middle::privacy::AccessLevels;
use rustc::util::nodemap::DefIdSet;
use std::mem;

use clean::{self, GetDefId, Item};
use fold;
use fold::StripItem;

mod collapse_docs;
pub use self::collapse_docs::COLLAPSE_DOCS;

mod strip_hidden;
pub use self::strip_hidden::STRIP_HIDDEN;

mod strip_private;
pub use self::strip_private::STRIP_PRIVATE;

mod strip_priv_imports;
pub use self::strip_priv_imports::STRIP_PRIV_IMPORTS;

mod unindent_comments;
pub use self::unindent_comments::UNINDENT_COMMENTS;

mod propagate_doc_cfg;
pub use self::propagate_doc_cfg::PROPAGATE_DOC_CFG;

#[derive(Copy, Clone, Debug)]
pub enum Pass {
    LatePass {
        name: &'static str,
        pass: fn(clean::Crate) -> clean::Crate,
        description: &'static str,
    }
}

impl Pass {
    pub const fn late(name: &'static str,
                      pass: fn(clean::Crate) -> clean::Crate,
                      description: &'static str) -> Pass {
        Pass::LatePass { name, pass, description }
    }

    pub fn name(self) -> &'static str {
        match self {
            Pass::LatePass { name, .. } => name,
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Pass::LatePass { description, .. } => description,
        }
    }

    pub fn late_fn(self) -> Option<fn(clean::Crate) -> clean::Crate> {
        match self {
            Pass::LatePass { pass, .. } => Some(pass),
        }
    }
}

pub const PASSES: &'static [Pass] = &[
    STRIP_HIDDEN,
    UNINDENT_COMMENTS,
    COLLAPSE_DOCS,
    STRIP_PRIVATE,
    STRIP_PRIV_IMPORTS,
    PROPAGATE_DOC_CFG,
];

pub const DEFAULT_PASSES: &'static [&'static str] = &[
    "strip-hidden",
    "strip-private",
    "collapse-docs",
    "unindent-comments",
    "propagate-doc-cfg",
];

pub const DEFAULT_PRIVATE_PASSES: &'static [&'static str] = &[
    "strip-priv-imports",
    "collapse-docs",
    "unindent-comments",
    "propagate-doc-cfg",
];

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum DefaultPassOption {
    Default,
    Private,
    None,
}

pub fn defaults(default_set: DefaultPassOption) -> &'static [&'static str] {
    match default_set {
        DefaultPassOption::Default => DEFAULT_PASSES,
        DefaultPassOption::Private => DEFAULT_PRIVATE_PASSES,
        DefaultPassOption::None => &[],
    }
}

struct Stripper<'a> {
    retained: &'a mut DefIdSet,
    access_levels: &'a AccessLevels<DefId>,
    update_retained: bool,
}

impl<'a> fold::DocFolder for Stripper<'a> {
    fn fold_item(&mut self, i: Item) -> Option<Item> {
        match i.inner {
            clean::StrippedItem(..) => {
                // We need to recurse into stripped modules to strip things
                // like impl methods but when doing so we must not add any
                // items to the `retained` set.
                let old = mem::replace(&mut self.update_retained, false);
                let ret = self.fold_item_recur(i);
                self.update_retained = old;
                return ret;
            }
            // These items can all get re-exported
            clean::ExistentialItem(..)
            | clean::TypedefItem(..)
            | clean::StaticItem(..)
            | clean::StructItem(..)
            | clean::EnumItem(..)
            | clean::TraitItem(..)
            | clean::FunctionItem(..)
            | clean::VariantItem(..)
            | clean::MethodItem(..)
            | clean::ForeignFunctionItem(..)
            | clean::ForeignStaticItem(..)
            | clean::ConstantItem(..)
            | clean::UnionItem(..)
            | clean::AssociatedConstItem(..)
            | clean::ForeignTypeItem => {
                if i.def_id.is_local() {
                    if !self.access_levels.is_exported(i.def_id) {
                        return None;
                    }
                }
            }

            clean::StructFieldItem(..) => {
                if i.visibility != Some(clean::Public) {
                    return StripItem(i).strip();
                }
            }

            clean::ModuleItem(..) => {
                if i.def_id.is_local() && i.visibility != Some(clean::Public) {
                    let old = mem::replace(&mut self.update_retained, false);
                    let ret = StripItem(self.fold_item_recur(i).unwrap()).strip();
                    self.update_retained = old;
                    return ret;
                }
            }

            // handled in the `strip-priv-imports` pass
            clean::ExternCrateItem(..) | clean::ImportItem(..) => {}

            clean::ImplItem(..) => {}

            // tymethods/macros have no control over privacy
            clean::MacroItem(..) | clean::TyMethodItem(..) => {}

            // Primitives are never stripped
            clean::PrimitiveItem(..) => {}

            // Associated types are never stripped
            clean::AssociatedTypeItem(..) => {}

            // Keywords are never stripped
            clean::KeywordItem(..) => {}
        }

        let fastreturn = match i.inner {
            // nothing left to do for traits (don't want to filter their
            // methods out, visibility controlled by the trait)
            clean::TraitItem(..) => true,

            // implementations of traits are always public.
            clean::ImplItem(ref imp) if imp.trait_.is_some() => true,
            // Struct variant fields have inherited visibility
            clean::VariantItem(clean::Variant {
                kind: clean::VariantKind::Struct(..),
            }) => true,
            _ => false,
        };

        let i = if fastreturn {
            if self.update_retained {
                self.retained.insert(i.def_id);
            }
            return Some(i);
        } else {
            self.fold_item_recur(i)
        };

        if let Some(ref i) = i {
            if self.update_retained {
                self.retained.insert(i.def_id);
            }
        }
        i
    }
}

// This stripper discards all impls which reference stripped items
struct ImplStripper<'a> {
    retained: &'a DefIdSet,
}

impl<'a> fold::DocFolder for ImplStripper<'a> {
    fn fold_item(&mut self, i: Item) -> Option<Item> {
        if let clean::ImplItem(ref imp) = i.inner {
            // emptied none trait impls can be stripped
            if imp.trait_.is_none() && imp.items.is_empty() {
                return None;
            }
            if let Some(did) = imp.for_.def_id() {
                if did.is_local() && !imp.for_.is_generic() && !self.retained.contains(&did) {
                    return None;
                }
            }
            if let Some(did) = imp.trait_.def_id() {
                if did.is_local() && !self.retained.contains(&did) {
                    return None;
                }
            }
            if let Some(generics) = imp.trait_.as_ref().and_then(|t| t.generics()) {
                for typaram in generics {
                    if let Some(did) = typaram.def_id() {
                        if did.is_local() && !self.retained.contains(&did) {
                            return None;
                        }
                    }
                }
            }
        }
        self.fold_item_recur(i)
    }
}

// This stripper discards all private import statements (`use`, `extern crate`)
struct ImportStripper;
impl fold::DocFolder for ImportStripper {
    fn fold_item(&mut self, i: Item) -> Option<Item> {
        match i.inner {
            clean::ExternCrateItem(..) | clean::ImportItem(..)
                if i.visibility != Some(clean::Public) =>
            {
                None
            }
            _ => self.fold_item_recur(i),
        }
    }
}
