// Copyright 2012-2014 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use middle::mem_categorization::Typer;
use middle::ty;
use util::ppaux::{ty_to_string};
use util::ppaux::UserString;

use syntax::ast::*;
use syntax::codemap::Span;
use syntax::visit::Visitor;
use syntax::visit;

// Kind analysis pass. This pass does some ad-hoc checks that are more
// convenient to do after type checking is complete and all checks are
// known. These are generally related to the builtin bounds `Copy` and
// `Sized`. Note that many of the builtin bound properties that used
// to be checked here are actually checked by trait checking these
// days.

pub struct Context<'a,'tcx:'a> {
    tcx: &'a ty::ctxt<'tcx>,
}

impl<'a, 'tcx, 'v> Visitor<'v> for Context<'a, 'tcx> {
    fn visit_fn(&mut self, fk: visit::FnKind, fd: &'v FnDecl,
                b: &'v Block, s: Span, n: NodeId) {
        check_fn(self, fk, fd, b, s, n);
    }

    fn visit_ty(&mut self, t: &Ty) {
        check_ty(self, t);
    }
}

pub fn check_crate(tcx: &ty::ctxt) {
    let mut ctx = Context {
        tcx: tcx,
    };
    visit::walk_crate(&mut ctx, tcx.map.krate());
    tcx.sess.abort_if_errors();
}

// Yields the appropriate function to check the kind of closed over
// variables. `id` is the NodeId for some expression that creates the
// closure.
fn with_appropriate_checker(cx: &Context,
                            id: NodeId,
                            fn_span: Span,
                            b: |checker: |&Context, &ty::Freevar||) {
    fn check_for_uniq(cx: &Context,
                      fn_span: Span,
                      fv: &ty::Freevar,
                      bounds: ty::BuiltinBounds) {
        // all captured data must be owned, regardless of whether it is
        // moved in or copied in.
        let id = fv.def.def_id().node;
        let var_t = ty::node_id_to_type(cx.tcx, id);

        check_freevar_bounds(cx, fn_span, fv.span, var_t, bounds, None);
    }

    fn check_for_block(cx: &Context,
                       fn_span: Span,
                       fn_id: NodeId,
                       fv: &ty::Freevar,
                       bounds: ty::BuiltinBounds) {
        let id = fv.def.def_id().node;
        let var_t = ty::node_id_to_type(cx.tcx, id);
        let upvar_id = ty::UpvarId { var_id: id, closure_expr_id: fn_id };
        let upvar_borrow = cx.tcx.upvar_borrow(upvar_id);
        let implicit_borrowed_type =
            ty::mk_rptr(cx.tcx,
                        upvar_borrow.region,
                        ty::mt { mutbl: upvar_borrow.kind.to_mutbl_lossy(),
                                 ty: var_t });
        check_freevar_bounds(cx, fn_span, fv.span, implicit_borrowed_type,
                             bounds, Some(var_t));
    }

    fn check_for_bare(cx: &Context, fv: &ty::Freevar) {
        span_err!(cx.tcx.sess, fv.span, E0143,
                  "can't capture dynamic environment in a fn item; \
                   use the || {} closure form instead", "{ ... }");
    } // same check is done in resolve.rs, but shouldn't be done

    let fty = ty::node_id_to_type(cx.tcx, id);
    match ty::get(fty).sty {
        ty::ty_closure(box ty::ClosureTy {
            store: ty::UniqTraitStore,
            bounds: bounds,
            ..
        }) => {
            b(|cx, fv| check_for_uniq(cx, fn_span, fv,
                                      bounds.builtin_bounds))
        }

        ty::ty_closure(box ty::ClosureTy {
            store: ty::RegionTraitStore(..), bounds, ..
        }) => {
            b(|cx, fv| check_for_block(cx, fn_span, id, fv,
                                       bounds.builtin_bounds))
        }

        ty::ty_bare_fn(_) => {
            b(check_for_bare)
        }

        ty::ty_unboxed_closure(..) => {}

        ref s => {
            cx.tcx.sess.bug(format!("expect fn type in kind checker, not \
                                     {:?}",
                                    s).as_slice());
        }
    }
}

// Check that the free variables used in a shared/sendable closure conform
// to the copy/move kind bounds. Then recursively check the function body.
fn check_fn(
    cx: &mut Context,
    fk: visit::FnKind,
    decl: &FnDecl,
    body: &Block,
    sp: Span,
    fn_id: NodeId) {

    // <Check kinds on free variables:
    with_appropriate_checker(cx, fn_id, sp, |chk| {
        ty::with_freevars(cx.tcx, fn_id, |freevars| {
            for fv in freevars.iter() {
                chk(cx, fv);
            }
        });
    });

    match fk {
        visit::FkFnBlock(..) => {
            visit::walk_fn(cx, fk, decl, body, sp)
        }
        visit::FkItemFn(..) | visit::FkMethod(..) => {
            visit::walk_fn(cx, fk, decl, body, sp);
        }
    }
}

fn check_ty(cx: &mut Context, aty: &Ty) {
    match aty.node {
        TyPath(_, _, id) => {
            match cx.tcx.item_substs.borrow().find(&id) {
                None => {}
                Some(ref item_substs) => {
                    let def_map = cx.tcx.def_map.borrow();
                    let did = def_map.get_copy(&id).def_id();
                    let generics = ty::lookup_item_type(cx.tcx, did).generics;
                    for def in generics.types.iter() {
                        let ty = *item_substs.substs.types.get(def.space,
                                                               def.index);
                        check_typaram_bounds(cx, aty.span, ty, def);
                    }
                }
            }
        }
        _ => {}
    }

    visit::walk_ty(cx, aty);
}

// Calls "any_missing" if any bounds were missing.
pub fn check_builtin_bounds(cx: &Context,
                            ty: ty::t,
                            bounds: ty::BuiltinBounds,
                            any_missing: |ty::BuiltinBounds|) {
    let kind = ty::type_contents(cx.tcx, ty);
    let mut missing = ty::empty_builtin_bounds();
    for bound in bounds.iter() {
        if !kind.meets_builtin_bound(cx.tcx, bound) {
            missing.add(bound);
        }
    }
    if !missing.is_empty() {
        any_missing(missing);
    }
}

pub fn check_typaram_bounds(cx: &Context,
                            sp: Span,
                            ty: ty::t,
                            type_param_def: &ty::TypeParameterDef) {
    check_builtin_bounds(cx,
                         ty,
                         type_param_def.bounds.builtin_bounds,
                         |missing| {
        span_err!(cx.tcx.sess, sp, E0144,
                  "instantiating a type parameter with an incompatible type \
                   `{}`, which does not fulfill `{}`",
                   ty_to_string(cx.tcx, ty),
                   missing.user_string(cx.tcx));
    });
}

pub fn check_freevar_bounds(cx: &Context, fn_span: Span, sp: Span, ty: ty::t,
                            bounds: ty::BuiltinBounds, referenced_ty: Option<ty::t>)
{
    check_builtin_bounds(cx, ty, bounds, |missing| {
        // Will be Some if the freevar is implicitly borrowed (stack closure).
        // Emit a less mysterious error message in this case.
        match referenced_ty {
            Some(rty) => {
                span_err!(cx.tcx.sess, sp, E0145,
                    "cannot implicitly borrow variable of type `{}` in a \
                     bounded stack closure (implicit reference does not fulfill `{}`)",
                    ty_to_string(cx.tcx, rty), missing.user_string(cx.tcx));
            }
            None => {
                span_err!(cx.tcx.sess, sp, E0146,
                    "cannot capture variable of type `{}`, which does \
                     not fulfill `{}`, in a bounded closure",
                    ty_to_string(cx.tcx, ty), missing.user_string(cx.tcx));
            }
        }
        span_note!(cx.tcx.sess, fn_span,
            "this closure's environment must satisfy `{}`",
            bounds.user_string(cx.tcx));
    });
}

