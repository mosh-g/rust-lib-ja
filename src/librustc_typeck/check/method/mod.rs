// Copyright 2014 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Method lookup: the secret sauce of Rust. See `README.md`.

use astconv::AstConv;
use check::FnCtxt;
use middle::def;
use middle::privacy::{AllPublic, DependsOn, LastPrivate, LastMod};
use middle::subst;
use middle::traits;
use middle::ty::{self, AsPredicate, ToPolyTraitRef, TraitRef};
use middle::infer;

use syntax::ast::DefId;
use syntax::ast;
use syntax::codemap::Span;

pub use self::MethodError::*;
pub use self::CandidateSource::*;

pub use self::suggest::{report_error, AllTraitsVec};

mod confirm;
mod probe;
mod suggest;

pub enum MethodError<'tcx> {
    // Did not find an applicable method, but we did find various near-misses that may work.
    NoMatch(NoMatchData<'tcx>),

    // Multiple methods might apply.
    Ambiguity(Vec<CandidateSource>),

    // Using a `Fn`/`FnMut`/etc method on a raw closure type before we have inferred its kind.
    ClosureAmbiguity(/* DefId of fn trait */ ast::DefId),
}

// Contains a list of static methods that may apply, a list of unsatisfied trait predicates which
// could lead to matches if satisfied, and a list of not-in-scope traits which may work.
pub struct NoMatchData<'tcx> {
    pub static_candidates: Vec<CandidateSource>,
    pub unsatisfied_predicates: Vec<TraitRef<'tcx>>,
    pub out_of_scope_traits: Vec<ast::DefId>,
    pub mode: probe::Mode
}

impl<'tcx> NoMatchData<'tcx> {
    pub fn new(static_candidates: Vec<CandidateSource>,
               unsatisfied_predicates: Vec<TraitRef<'tcx>>,
               out_of_scope_traits: Vec<ast::DefId>,
               mode: probe::Mode) -> Self {
        NoMatchData {
            static_candidates: static_candidates,
            unsatisfied_predicates: unsatisfied_predicates,
            out_of_scope_traits: out_of_scope_traits,
            mode: mode
        }
    }
}

// A pared down enum describing just the places from which a method
// candidate can arise. Used for error reporting only.
#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CandidateSource {
    ImplSource(ast::DefId),
    TraitSource(/* trait id */ ast::DefId),
}

type ItemIndex = usize; // just for doc purposes

/// Determines whether the type `self_ty` supports a method name `method_name` or not.
pub fn exists<'a, 'tcx>(fcx: &FnCtxt<'a, 'tcx>,
                        span: Span,
                        method_name: ast::Name,
                        self_ty: ty::Ty<'tcx>,
                        call_expr_id: ast::NodeId)
                        -> bool
{
    let mode = probe::Mode::MethodCall;
    match probe::probe(fcx, span, mode, method_name, self_ty, call_expr_id) {
        Ok(..) => true,
        Err(NoMatch(..)) => false,
        Err(Ambiguity(..)) => true,
        Err(ClosureAmbiguity(..)) => true,
    }
}

/// Performs method lookup. If lookup is successful, it will return the callee and store an
/// appropriate adjustment for the self-expr. In some cases it may report an error (e.g., invoking
/// the `drop` method).
///
/// # Arguments
///
/// Given a method call like `foo.bar::<T1,...Tn>(...)`:
///
/// * `fcx`:                   the surrounding `FnCtxt` (!)
/// * `span`:                  the span for the method call
/// * `method_name`:           the name of the method being called (`bar`)
/// * `self_ty`:               the (unadjusted) type of the self expression (`foo`)
/// * `supplied_method_types`: the explicit method type parameters, if any (`T1..Tn`)
/// * `self_expr`:             the self expression (`foo`)
pub fn lookup<'a, 'tcx>(fcx: &FnCtxt<'a, 'tcx>,
                        span: Span,
                        method_name: ast::Name,
                        self_ty: ty::Ty<'tcx>,
                        supplied_method_types: Vec<ty::Ty<'tcx>>,
                        call_expr: &'tcx ast::Expr,
                        self_expr: &'tcx ast::Expr)
                        -> Result<ty::MethodCallee<'tcx>, MethodError<'tcx>>
{
    debug!("lookup(method_name={}, self_ty={:?}, call_expr={:?}, self_expr={:?})",
           method_name,
           self_ty,
           call_expr,
           self_expr);

    let mode = probe::Mode::MethodCall;
    let self_ty = fcx.infcx().resolve_type_vars_if_possible(&self_ty);
    let pick = try!(probe::probe(fcx, span, mode, method_name, self_ty, call_expr.id));
    Ok(confirm::confirm(fcx, span, self_expr, call_expr, self_ty, pick, supplied_method_types))
}

pub fn lookup_in_trait<'a, 'tcx>(fcx: &FnCtxt<'a, 'tcx>,
                                 span: Span,
                                 self_expr: Option<&ast::Expr>,
                                 m_name: ast::Name,
                                 trait_def_id: DefId,
                                 self_ty: ty::Ty<'tcx>,
                                 opt_input_types: Option<Vec<ty::Ty<'tcx>>>)
                                 -> Option<ty::MethodCallee<'tcx>>
{
    lookup_in_trait_adjusted(fcx, span, self_expr, m_name, trait_def_id,
                             0, false, self_ty, opt_input_types)
}

/// `lookup_in_trait_adjusted` is used for overloaded operators. It does a very narrow slice of
/// what the normal probe/confirm path does. In particular, it doesn't really do any probing: it
/// simply constructs an obligation for a particular trait with the given self-type and checks
/// whether that trait is implemented.
///
/// FIXME(#18741) -- It seems likely that we can consolidate some of this code with the other
/// method-lookup code. In particular, autoderef on index is basically identical to autoderef with
/// normal probes, except that the test also looks for built-in indexing. Also, the second half of
/// this method is basically the same as confirmation.
pub fn lookup_in_trait_adjusted<'a, 'tcx>(fcx: &FnCtxt<'a, 'tcx>,
                                          span: Span,
                                          self_expr: Option<&ast::Expr>,
                                          m_name: ast::Name,
                                          trait_def_id: DefId,
                                          autoderefs: usize,
                                          unsize: bool,
                                          self_ty: ty::Ty<'tcx>,
                                          opt_input_types: Option<Vec<ty::Ty<'tcx>>>)
                                          -> Option<ty::MethodCallee<'tcx>>
{
    debug!("lookup_in_trait_adjusted(self_ty={:?}, self_expr={:?}, m_name={}, trait_def_id={:?})",
           self_ty,
           self_expr,
           m_name,
           trait_def_id);

    let trait_def = ty::lookup_trait_def(fcx.tcx(), trait_def_id);

    let expected_number_of_input_types = trait_def.generics.types.len(subst::TypeSpace);
    let input_types = match opt_input_types {
        Some(input_types) => {
            assert_eq!(expected_number_of_input_types, input_types.len());
            input_types
        }

        None => {
            fcx.inh.infcx.next_ty_vars(expected_number_of_input_types)
        }
    };

    assert_eq!(trait_def.generics.types.len(subst::FnSpace), 0);
    assert!(trait_def.generics.regions.is_empty());

    // Construct a trait-reference `self_ty : Trait<input_tys>`
    let substs = subst::Substs::new_trait(input_types, Vec::new(), self_ty);
    let trait_ref = ty::TraitRef::new(trait_def_id, fcx.tcx().mk_substs(substs));

    // Construct an obligation
    let poly_trait_ref = trait_ref.to_poly_trait_ref();
    let obligation = traits::Obligation::misc(span,
                                              fcx.body_id,
                                              poly_trait_ref.as_predicate());

    // Now we want to know if this can be matched
    let mut selcx = traits::SelectionContext::new(fcx.infcx(), fcx);
    if !selcx.evaluate_obligation(&obligation) {
        debug!("--> Cannot match obligation");
        return None; // Cannot be matched, no such method resolution is possible.
    }

    // Trait must have a method named `m_name` and it should not have
    // type parameters or early-bound regions.
    let tcx = fcx.tcx();
    let (method_num, method_ty) = trait_item(tcx, trait_def_id, m_name)
            .and_then(|(idx, item)| item.as_opt_method().map(|m| (idx, m)))
            .unwrap();
    assert_eq!(method_ty.generics.types.len(subst::FnSpace), 0);
    assert_eq!(method_ty.generics.regions.len(subst::FnSpace), 0);

    debug!("lookup_in_trait_adjusted: method_num={} method_ty={:?}",
           method_num, method_ty);

    // Instantiate late-bound regions and substitute the trait
    // parameters into the method type to get the actual method type.
    //
    // NB: Instantiate late-bound regions first so that
    // `instantiate_type_scheme` can normalize associated types that
    // may reference those regions.
    let fn_sig = fcx.infcx().replace_late_bound_regions_with_fresh_var(span,
                                                                       infer::FnCall,
                                                                       &method_ty.fty.sig).0;
    let fn_sig = fcx.instantiate_type_scheme(span, trait_ref.substs, &fn_sig);
    let transformed_self_ty = fn_sig.inputs[0];
    let fty = ty::mk_bare_fn(tcx, None, tcx.mk_bare_fn(ty::BareFnTy {
        sig: ty::Binder(fn_sig),
        unsafety: method_ty.fty.unsafety,
        abi: method_ty.fty.abi.clone(),
    }));

    debug!("lookup_in_trait_adjusted: matched method fty={:?} obligation={:?}",
           fty,
           obligation);

    // Register obligations for the parameters.  This will include the
    // `Self` parameter, which in turn has a bound of the main trait,
    // so this also effectively registers `obligation` as well.  (We
    // used to register `obligation` explicitly, but that resulted in
    // double error messages being reported.)
    //
    // Note that as the method comes from a trait, it should not have
    // any late-bound regions appearing in its bounds.
    let method_bounds = fcx.instantiate_bounds(span, trait_ref.substs, &method_ty.predicates);
    assert!(!method_bounds.has_escaping_regions());
    fcx.add_obligations_for_parameters(
        traits::ObligationCause::misc(span, fcx.body_id),
        &method_bounds);

    // FIXME(#18653) -- Try to resolve obligations, giving us more
    // typing information, which can sometimes be needed to avoid
    // pathological region inference failures.
    fcx.select_new_obligations();

    // Insert any adjustments needed (always an autoref of some mutability).
    match self_expr {
        None => { }

        Some(self_expr) => {
            debug!("lookup_in_trait_adjusted: inserting adjustment if needed \
                   (self-id={}, autoderefs={}, unsize={}, explicit_self={:?})",
                   self_expr.id, autoderefs, unsize,
                   method_ty.explicit_self);

            match method_ty.explicit_self {
                ty::ByValueExplicitSelfCategory => {
                    // Trait method is fn(self), no transformation needed.
                    assert!(!unsize);
                    fcx.write_autoderef_adjustment(self_expr.id, autoderefs);
                }

                ty::ByReferenceExplicitSelfCategory(..) => {
                    // Trait method is fn(&self) or fn(&mut self), need an
                    // autoref. Pull the region etc out of the type of first argument.
                    match transformed_self_ty.sty {
                        ty::TyRef(region, ty::mt { mutbl, ty: _ }) => {
                            fcx.write_adjustment(self_expr.id,
                                ty::AdjustDerefRef(ty::AutoDerefRef {
                                    autoderefs: autoderefs,
                                    autoref: Some(ty::AutoPtr(region, mutbl)),
                                    unsize: if unsize {
                                        Some(transformed_self_ty)
                                    } else {
                                        None
                                    }
                                }));
                        }

                        _ => {
                            fcx.tcx().sess.span_bug(
                                span,
                                &format!(
                                    "trait method is &self but first arg is: {}",
                                    transformed_self_ty));
                        }
                    }
                }

                _ => {
                    fcx.tcx().sess.span_bug(
                        span,
                        &format!(
                            "unexpected explicit self type in operator method: {:?}",
                            method_ty.explicit_self));
                }
            }
        }
    }

    let callee = ty::MethodCallee {
        origin: ty::MethodTypeParam(ty::MethodParam{trait_ref: trait_ref.clone(),
                                                    method_num: method_num,
                                                    impl_def_id: None}),
        ty: fty,
        substs: trait_ref.substs.clone()
    };

    debug!("callee = {:?}", callee);

    Some(callee)
}

pub fn resolve_ufcs<'a, 'tcx>(fcx: &FnCtxt<'a, 'tcx>,
                              span: Span,
                              method_name: ast::Name,
                              self_ty: ty::Ty<'tcx>,
                              expr_id: ast::NodeId)
                              -> Result<(def::Def, LastPrivate), MethodError<'tcx>>
{
    let mode = probe::Mode::Path;
    let pick = try!(probe::probe(fcx, span, mode, method_name, self_ty, expr_id));
    let def_id = pick.item.def_id();
    let mut lp = LastMod(AllPublic);
    let provenance = match pick.kind {
        probe::InherentImplPick(impl_def_id) => {
            if pick.item.vis() != ast::Public {
                lp = LastMod(DependsOn(def_id));
            }
            def::FromImpl(impl_def_id)
        }
        _ => def::FromTrait(pick.item.container().id())
    };
    let def_result = match pick.item {
        ty::ImplOrTraitItem::MethodTraitItem(..) => def::DefMethod(def_id, provenance),
        ty::ImplOrTraitItem::ConstTraitItem(..) => def::DefAssociatedConst(def_id, provenance),
        ty::ImplOrTraitItem::TypeTraitItem(..) => {
            fcx.tcx().sess.span_bug(span, "resolve_ufcs: probe picked associated type");
        }
    };
    Ok((def_result, lp))
}


/// Find item with name `item_name` defined in `trait_def_id` and return it, along with its
/// index (or `None`, if no such item).
fn trait_item<'tcx>(tcx: &ty::ctxt<'tcx>,
                    trait_def_id: ast::DefId,
                    item_name: ast::Name)
                    -> Option<(usize, ty::ImplOrTraitItem<'tcx>)>
{
    let trait_items = ty::trait_items(tcx, trait_def_id);
    trait_items
        .iter()
        .enumerate()
        .find(|&(_, ref item)| item.name() == item_name)
        .map(|(num, item)| (num, (*item).clone()))
}

fn impl_item<'tcx>(tcx: &ty::ctxt<'tcx>,
                   impl_def_id: ast::DefId,
                   item_name: ast::Name)
                   -> Option<ty::ImplOrTraitItem<'tcx>>
{
    let impl_items = tcx.impl_items.borrow();
    let impl_items = impl_items.get(&impl_def_id).unwrap();
    impl_items
        .iter()
        .map(|&did| ty::impl_or_trait_item(tcx, did.def_id()))
        .find(|m| m.name() == item_name)
}
