// Copyright 2014 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

// This compiler pass detects static items that refer to themselves
// recursively.

use ast_map;
use session::Session;
use middle::def::{DefStatic, DefConst, DefAssociatedConst, DefVariant, DefMap};
use util::nodemap::NodeMap;

use syntax::{ast, ast_util};
use syntax::codemap::Span;
use syntax::visit::Visitor;
use syntax::visit;

use std::cell::RefCell;

struct CheckCrateVisitor<'a, 'ast: 'a> {
    sess: &'a Session,
    def_map: &'a DefMap,
    ast_map: &'a ast_map::Map<'ast>,
    discriminant_map: RefCell<NodeMap<Option<&'ast ast::Expr>>>,
}

impl<'a, 'ast: 'a> Visitor<'ast> for CheckCrateVisitor<'a, 'ast> {
    fn visit_item(&mut self, it: &'ast ast::Item) {
        match it.node {
            ast::ItemStatic(..) |
            ast::ItemConst(..) => {
                let mut recursion_visitor =
                    CheckItemRecursionVisitor::new(self, &it.span);
                recursion_visitor.visit_item(it);
            },
            ast::ItemEnum(ref enum_def, ref generics) => {
                // We could process the whole enum, but handling the variants
                // with discriminant expressions one by one gives more specific,
                // less redundant output.
                for variant in &enum_def.variants {
                    if let Some(_) = variant.node.disr_expr {
                        let mut recursion_visitor =
                            CheckItemRecursionVisitor::new(self, &variant.span);
                        recursion_visitor.populate_enum_discriminants(enum_def);
                        recursion_visitor.visit_variant(variant, generics);
                    }
                }
            }
            _ => {}
        }
        visit::walk_item(self, it)
    }

    fn visit_trait_item(&mut self, ti: &'ast ast::TraitItem) {
        match ti.node {
            ast::ConstTraitItem(_, ref default) => {
                if let Some(_) = *default {
                    let mut recursion_visitor =
                        CheckItemRecursionVisitor::new(self, &ti.span);
                    recursion_visitor.visit_trait_item(ti);
                }
            }
            _ => {}
        }
        visit::walk_trait_item(self, ti)
    }

    fn visit_impl_item(&mut self, ii: &'ast ast::ImplItem) {
        match ii.node {
            ast::ConstImplItem(..) => {
                let mut recursion_visitor =
                    CheckItemRecursionVisitor::new(self, &ii.span);
                recursion_visitor.visit_impl_item(ii);
            }
            _ => {}
        }
        visit::walk_impl_item(self, ii)
    }
}

pub fn check_crate<'ast>(sess: &Session,
                         krate: &'ast ast::Crate,
                         def_map: &DefMap,
                         ast_map: &ast_map::Map<'ast>) {
    let mut visitor = CheckCrateVisitor {
        sess: sess,
        def_map: def_map,
        ast_map: ast_map,
        discriminant_map: RefCell::new(NodeMap()),
    };
    visit::walk_crate(&mut visitor, krate);
    sess.abort_if_errors();
}

struct CheckItemRecursionVisitor<'a, 'ast: 'a> {
    root_span: &'a Span,
    sess: &'a Session,
    ast_map: &'a ast_map::Map<'ast>,
    def_map: &'a DefMap,
    discriminant_map: &'a RefCell<NodeMap<Option<&'ast ast::Expr>>>,
    idstack: Vec<ast::NodeId>,
}

impl<'a, 'ast: 'a> CheckItemRecursionVisitor<'a, 'ast> {
    fn new(v: &'a CheckCrateVisitor<'a, 'ast>, span: &'a Span)
           -> CheckItemRecursionVisitor<'a, 'ast> {
        CheckItemRecursionVisitor {
            root_span: span,
            sess: v.sess,
            ast_map: v.ast_map,
            def_map: v.def_map,
            discriminant_map: &v.discriminant_map,
            idstack: Vec::new(),
        }
    }
    fn with_item_id_pushed<F>(&mut self, id: ast::NodeId, f: F)
          where F: Fn(&mut Self) {
        if self.idstack.iter().any(|x| *x == id) {
            span_err!(self.sess, *self.root_span, E0265, "recursive constant");
            return;
        }
        self.idstack.push(id);
        f(self);
        self.idstack.pop();
    }
    // If a variant has an expression specifying its discriminant, then it needs
    // to be checked just like a static or constant. However, if there are more
    // variants with no explicitly specified discriminant, those variants will
    // increment the same expression to get their values.
    //
    // So for every variant, we need to track whether there is an expression
    // somewhere in the enum definition that controls its discriminant. We do
    // this by starting from the end and searching backward.
    fn populate_enum_discriminants(&self, enum_definition: &'ast ast::EnumDef) {
        // Get the map, and return if we already processed this enum or if it
        // has no variants.
        let mut discriminant_map = self.discriminant_map.borrow_mut();
        match enum_definition.variants.first() {
            None => { return; }
            Some(variant) if discriminant_map.contains_key(&variant.node.id) => {
                return;
            }
            _ => {}
        }

        // Go through all the variants.
        let mut variant_stack: Vec<ast::NodeId> = Vec::new();
        for variant in enum_definition.variants.iter().rev() {
            variant_stack.push(variant.node.id);
            // When we find an expression, every variant currently on the stack
            // is affected by that expression.
            if let Some(ref expr) = variant.node.disr_expr {
                for id in &variant_stack {
                    discriminant_map.insert(*id, Some(expr));
                }
                variant_stack.clear()
            }
        }
        // If we are at the top, that always starts at 0, so any variant on the
        // stack has a default value and does not need to be checked.
        for id in &variant_stack {
            discriminant_map.insert(*id, None);
        }
    }
}

impl<'a, 'ast: 'a> Visitor<'ast> for CheckItemRecursionVisitor<'a, 'ast> {
    fn visit_item(&mut self, it: &'ast ast::Item) {
        self.with_item_id_pushed(it.id, |v| visit::walk_item(v, it));
    }

    fn visit_enum_def(&mut self, enum_definition: &'ast ast::EnumDef,
                      generics: &'ast ast::Generics) {
        self.populate_enum_discriminants(enum_definition);
        visit::walk_enum_def(self, enum_definition, generics);
    }

    fn visit_variant(&mut self, variant: &'ast ast::Variant,
                     _: &'ast ast::Generics) {
        let variant_id = variant.node.id;
        let maybe_expr;
        if let Some(get_expr) = self.discriminant_map.borrow().get(&variant_id) {
            // This is necessary because we need to let the `discriminant_map`
            // borrow fall out of scope, so that we can reborrow farther down.
            maybe_expr = (*get_expr).clone();
        } else {
            self.sess.span_bug(variant.span,
                               "`check_static_recursion` attempted to visit \
                                variant with unknown discriminant")
        }
        // If `maybe_expr` is `None`, that's because no discriminant is
        // specified that affects this variant. Thus, no risk of recursion.
        if let Some(expr) = maybe_expr {
            self.with_item_id_pushed(expr.id, |v| visit::walk_expr(v, expr));
        }
    }

    fn visit_trait_item(&mut self, ti: &'ast ast::TraitItem) {
        self.with_item_id_pushed(ti.id, |v| visit::walk_trait_item(v, ti));
    }

    fn visit_impl_item(&mut self, ii: &'ast ast::ImplItem) {
        self.with_item_id_pushed(ii.id, |v| visit::walk_impl_item(v, ii));
    }

    fn visit_expr(&mut self, e: &'ast ast::Expr) {
        match e.node {
            ast::ExprPath(..) => {
                match self.def_map.borrow().get(&e.id).map(|d| d.base_def) {
                    Some(DefStatic(def_id, _)) |
                    Some(DefAssociatedConst(def_id, _)) |
                    Some(DefConst(def_id))
                           if ast_util::is_local(def_id) => {
                        match self.ast_map.get(def_id.node) {
                          ast_map::NodeItem(item) =>
                            self.visit_item(item),
                          ast_map::NodeTraitItem(item) =>
                            self.visit_trait_item(item),
                          ast_map::NodeImplItem(item) =>
                            self.visit_impl_item(item),
                          ast_map::NodeForeignItem(_) => {},
                          _ => {
                              self.sess.span_bug(
                                  e.span,
                                  &format!("expected item, found {}",
                                           self.ast_map.node_to_string(def_id.node)));
                          }
                        }
                    }
                    // For variants, we only want to check expressions that
                    // affect the specific variant used, but we need to check
                    // the whole enum definition to see what expression that
                    // might be (if any).
                    Some(DefVariant(enum_id, variant_id, false))
                           if ast_util::is_local(enum_id) => {
                        if let ast::ItemEnum(ref enum_def, ref generics) =
                               self.ast_map.expect_item(enum_id.local_id()).node {
                            self.populate_enum_discriminants(enum_def);
                            let variant = self.ast_map.expect_variant(variant_id.local_id());
                            self.visit_variant(variant, generics);
                        } else {
                            self.sess.span_bug(e.span,
                                "`check_static_recursion` found \
                                 non-enum in DefVariant");
                        }
                    }
                    _ => ()
                }
            },
            _ => ()
        }
        visit::walk_expr(self, e);
    }
}
