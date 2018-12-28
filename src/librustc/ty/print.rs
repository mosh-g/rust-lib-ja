use crate::hir::def::Namespace;
use crate::hir::map::DefPathData;
use crate::hir::def_id::{CrateNum, DefId, CRATE_DEF_INDEX, LOCAL_CRATE};
use crate::ty::{self, DefIdTree, Ty, TyCtxt, TypeFoldable};
use crate::ty::subst::{Kind, Subst, SubstsRef, UnpackedKind};
use crate::middle::cstore::{ExternCrate, ExternCrateSource};
use syntax::ast;
use syntax::symbol::{keywords, Symbol};

use rustc_data_structures::fx::FxHashSet;
use syntax::symbol::InternedString;

use std::cell::Cell;
use std::fmt::{self, Write as _};
use std::iter;
use std::ops::Deref;

thread_local! {
    static FORCE_ABSOLUTE: Cell<bool> = Cell::new(false);
    static FORCE_IMPL_FILENAME_LINE: Cell<bool> = Cell::new(false);
    static SHOULD_PREFIX_WITH_CRATE: Cell<bool> = Cell::new(false);
}

/// Enforces that def_path_str always returns an absolute path and
/// also enables "type-based" impl paths. This is used when building
/// symbols that contain types, where we want the crate name to be
/// part of the symbol.
pub fn with_forced_absolute_paths<F: FnOnce() -> R, R>(f: F) -> R {
    FORCE_ABSOLUTE.with(|force| {
        let old = force.get();
        force.set(true);
        let result = f();
        force.set(old);
        result
    })
}

/// Force us to name impls with just the filename/line number. We
/// normally try to use types. But at some points, notably while printing
/// cycle errors, this can result in extra or suboptimal error output,
/// so this variable disables that check.
pub fn with_forced_impl_filename_line<F: FnOnce() -> R, R>(f: F) -> R {
    FORCE_IMPL_FILENAME_LINE.with(|force| {
        let old = force.get();
        force.set(true);
        let result = f();
        force.set(old);
        result
    })
}

/// Adds the `crate::` prefix to paths where appropriate.
pub fn with_crate_prefix<F: FnOnce() -> R, R>(f: F) -> R {
    SHOULD_PREFIX_WITH_CRATE.with(|flag| {
        let old = flag.get();
        flag.set(true);
        let result = f();
        flag.set(old);
        result
    })
}

// FIXME(eddyb) this module uses `pub(crate)` for things used only
// from `ppaux` - when that is removed, they can be re-privatized.

struct LateBoundRegionNameCollector(FxHashSet<InternedString>);
impl<'tcx> ty::fold::TypeVisitor<'tcx> for LateBoundRegionNameCollector {
    fn visit_region(&mut self, r: ty::Region<'tcx>) -> bool {
        match *r {
            ty::ReLateBound(_, ty::BrNamed(_, name)) => {
                self.0.insert(name);
            },
            _ => {},
        }
        r.super_visit_with(self)
    }
}

pub struct PrintCx<'a, 'gcx, 'tcx, P> {
    pub tcx: TyCtxt<'a, 'gcx, 'tcx>,
    pub printer: P,
    pub(crate) is_debug: bool,
    pub(crate) is_verbose: bool,
    pub(crate) identify_regions: bool,
    pub(crate) used_region_names: Option<FxHashSet<InternedString>>,
    pub(crate) region_index: usize,
    pub(crate) binder_depth: usize,
}

// HACK(eddyb) this is solely for `self: &mut PrintCx<Self>`, e.g. to
// implement traits on the printer and call the methods on the context.
impl<P> Deref for PrintCx<'_, '_, '_, P> {
    type Target = P;
    fn deref(&self) -> &P {
        &self.printer
    }
}

impl<P> PrintCx<'a, 'gcx, 'tcx, P> {
    pub fn new(tcx: TyCtxt<'a, 'gcx, 'tcx>, printer: P) -> Self {
        PrintCx {
            tcx,
            printer,
            is_debug: false,
            is_verbose: tcx.sess.verbose(),
            identify_regions: tcx.sess.opts.debugging_opts.identify_regions,
            used_region_names: None,
            region_index: 0,
            binder_depth: 0,
        }
    }

    pub(crate) fn with<R>(printer: P, f: impl FnOnce(PrintCx<'_, '_, '_, P>) -> R) -> R {
        ty::tls::with(|tcx| f(PrintCx::new(tcx, printer)))
    }
    pub(crate) fn prepare_late_bound_region_info<T>(&mut self, value: &ty::Binder<T>)
    where T: TypeFoldable<'tcx>
    {
        let mut collector = LateBoundRegionNameCollector(Default::default());
        value.visit_with(&mut collector);
        self.used_region_names = Some(collector.0);
        self.region_index = 0;
    }
}

pub trait Print<'tcx, P> {
    type Output;

    fn print(&self, cx: &mut PrintCx<'_, '_, 'tcx, P>) -> Self::Output;
    fn print_display(&self, cx: &mut PrintCx<'_, '_, 'tcx, P>) -> Self::Output {
        let old_debug = cx.is_debug;
        cx.is_debug = false;
        let result = self.print(cx);
        cx.is_debug = old_debug;
        result
    }
    fn print_debug(&self, cx: &mut PrintCx<'_, '_, 'tcx, P>) -> Self::Output {
        let old_debug = cx.is_debug;
        cx.is_debug = true;
        let result = self.print(cx);
        cx.is_debug = old_debug;
        result
    }
}

pub trait Printer: Sized {
    type Path;

    #[must_use]
    fn print_def_path(
        self: &mut PrintCx<'_, '_, 'tcx, Self>,
        def_id: DefId,
        substs: Option<SubstsRef<'tcx>>,
        ns: Namespace,
        projections: impl Iterator<Item = ty::ExistentialProjection<'tcx>>,
    ) -> Self::Path {
        self.default_print_def_path(def_id, substs, ns, projections)
    }
    #[must_use]
    fn print_impl_path(
        self: &mut PrintCx<'_, '_, 'tcx, Self>,
        impl_def_id: DefId,
        substs: Option<SubstsRef<'tcx>>,
        ns: Namespace,
        self_ty: Ty<'tcx>,
        trait_ref: Option<ty::TraitRef<'tcx>>,
    ) -> Self::Path {
        self.default_print_impl_path(impl_def_id, substs, ns, self_ty, trait_ref)
    }

    #[must_use]
    fn path_crate(self: &mut PrintCx<'_, '_, '_, Self>, cnum: CrateNum) -> Self::Path;
    #[must_use]
    fn path_qualified(
        self: &mut PrintCx<'_, '_, 'tcx, Self>,
        impl_prefix: Option<Self::Path>,
        self_ty: Ty<'tcx>,
        trait_ref: Option<ty::TraitRef<'tcx>>,
        ns: Namespace,
    ) -> Self::Path;
    #[must_use]
    fn path_append(
        self: &mut PrintCx<'_, '_, '_, Self>,
        path: Self::Path,
        text: &str,
    ) -> Self::Path;
    #[must_use]
    fn path_generic_args(
        self: &mut PrintCx<'_, '_, 'tcx, Self>,
        path: Self::Path,
        params: &[ty::GenericParamDef],
        substs: SubstsRef<'tcx>,
        ns: Namespace,
        projections: impl Iterator<Item = ty::ExistentialProjection<'tcx>>,
    ) -> Self::Path;
}

#[must_use]
pub struct PrettyPath {
    pub empty: bool,
}

/// Trait for printers that pretty-print using `fmt::Write` to the printer.
pub trait PrettyPrinter: Printer<Path = Result<PrettyPath, fmt::Error>> + fmt::Write {}

impl<'a, 'gcx, 'tcx> TyCtxt<'a, 'gcx, 'tcx> {
    // HACK(eddyb) get rid of `def_path_str` and/or pass `Namespace` explicitly always
    // (but also some things just print a `DefId` generally so maybe we need this?)
    fn guess_def_namespace(self, def_id: DefId) -> Namespace {
        match self.def_key(def_id).disambiguated_data.data {
            DefPathData::ValueNs(..) |
            DefPathData::EnumVariant(..) |
            DefPathData::Field(..) |
            DefPathData::AnonConst |
            DefPathData::ConstParam(..) |
            DefPathData::ClosureExpr |
            DefPathData::StructCtor => Namespace::ValueNS,

            DefPathData::MacroDef(..) => Namespace::MacroNS,

            _ => Namespace::TypeNS,
        }
    }

    /// Returns a string identifying this `DefId`. This string is
    /// suitable for user output. It is relative to the current crate
    /// root, unless with_forced_absolute_paths was used.
    pub fn def_path_str_with_substs_and_ns(
        self,
        def_id: DefId,
        substs: Option<SubstsRef<'tcx>>,
        ns: Namespace,
    ) -> String {
        debug!("def_path_str: def_id={:?}, substs={:?}, ns={:?}", def_id, substs, ns);
        let mut s = String::new();
        let _ = PrintCx::new(self, FmtPrinter { fmt: &mut s })
            .print_def_path(def_id, substs, ns, iter::empty());
        s
    }

    /// Returns a string identifying this `DefId`. This string is
    /// suitable for user output. It is relative to the current crate
    /// root, unless with_forced_absolute_paths was used.
    pub fn def_path_str(self, def_id: DefId) -> String {
        let ns = self.guess_def_namespace(def_id);
        debug!("def_path_str: def_id={:?}, ns={:?}", def_id, ns);
        let mut s = String::new();
        let _ = PrintCx::new(self, FmtPrinter { fmt: &mut s })
            .print_def_path(def_id, None, ns, iter::empty());
        s
    }

    /// Returns a string identifying this local node-id.
    // FIXME(eddyb) remove in favor of calling `def_path_str` directly.
    pub fn node_path_str(self, id: ast::NodeId) -> String {
        self.def_path_str(self.hir().local_def_id(id))
    }
}

impl<P: Printer> PrintCx<'a, 'gcx, 'tcx, P> {
    pub fn default_print_def_path(
        &mut self,
        def_id: DefId,
        substs: Option<SubstsRef<'tcx>>,
        ns: Namespace,
        projections: impl Iterator<Item = ty::ExistentialProjection<'tcx>>,
    ) -> P::Path {
        debug!("default_print_def_path: def_id={:?}, substs={:?}, ns={:?}", def_id, substs, ns);
        let key = self.tcx.def_key(def_id);
        debug!("default_print_def_path: key={:?}", key);

        match key.disambiguated_data.data {
            DefPathData::CrateRoot => {
                assert!(key.parent.is_none());
                self.path_crate(def_id.krate)
            }

            DefPathData::Impl => {
                let mut self_ty = self.tcx.type_of(def_id);
                if let Some(substs) = substs {
                    self_ty = self_ty.subst(self.tcx, substs);
                }

                let mut impl_trait_ref = self.tcx.impl_trait_ref(def_id);
                if let Some(substs) = substs {
                    impl_trait_ref = impl_trait_ref.subst(self.tcx, substs);
                }
                self.print_impl_path(def_id, substs, ns, self_ty, impl_trait_ref)
            }

            _ => {
                let generics = substs.map(|_| self.tcx.generics_of(def_id));
                let generics_parent = generics.as_ref().and_then(|g| g.parent);
                let parent_def_id = DefId { index: key.parent.unwrap(), ..def_id };
                let path = if let Some(generics_parent_def_id) = generics_parent {
                    assert_eq!(parent_def_id, generics_parent_def_id);

                    // FIXME(eddyb) try to move this into the parent's printing
                    // logic, instead of doing it when printing the child.
                    let parent_generics = self.tcx.generics_of(parent_def_id);
                    let parent_has_own_self =
                        parent_generics.has_self && parent_generics.parent_count == 0;
                    if let (Some(substs), true) = (substs, parent_has_own_self) {
                        let trait_ref = ty::TraitRef::new(parent_def_id, substs);
                        self.path_qualified(None, trait_ref.self_ty(), Some(trait_ref), ns)
                    } else {
                        self.print_def_path(parent_def_id, substs, ns, iter::empty())
                    }
                } else {
                    self.print_def_path(parent_def_id, None, ns, iter::empty())
                };
                let path = match key.disambiguated_data.data {
                    // Skip `::{{constructor}}` on tuple/unit structs.
                    DefPathData::StructCtor => path,

                    _ => {
                        self.path_append(
                            path,
                            &key.disambiguated_data.data.as_interned_str().as_str(),
                        )
                    }
                };

                if let (Some(generics), Some(substs)) = (generics, substs) {
                    let has_own_self = generics.has_self && generics.parent_count == 0;
                    let params = &generics.params[has_own_self as usize..];
                    self.path_generic_args(path, params, substs, ns, projections)
                } else {
                    path
                }
            }
        }
    }

    fn default_print_impl_path(
        &mut self,
        impl_def_id: DefId,
        _substs: Option<SubstsRef<'tcx>>,
        ns: Namespace,
        self_ty: Ty<'tcx>,
        impl_trait_ref: Option<ty::TraitRef<'tcx>>,
    ) -> P::Path {
        debug!("default_print_impl_path: impl_def_id={:?}, self_ty={}, impl_trait_ref={:?}",
               impl_def_id, self_ty, impl_trait_ref);

        // Decide whether to print the parent path for the impl.
        // Logically, since impls are global, it's never needed, but
        // users may find it useful. Currently, we omit the parent if
        // the impl is either in the same module as the self-type or
        // as the trait.
        let parent_def_id = self.tcx.parent(impl_def_id).unwrap();
        let in_self_mod = match characteristic_def_id_of_type(self_ty) {
            None => false,
            Some(ty_def_id) => self.tcx.parent(ty_def_id) == Some(parent_def_id),
        };
        let in_trait_mod = match impl_trait_ref {
            None => false,
            Some(trait_ref) => self.tcx.parent(trait_ref.def_id) == Some(parent_def_id),
        };

        let prefix_path = if !in_self_mod && !in_trait_mod {
            // If the impl is not co-located with either self-type or
            // trait-type, then fallback to a format that identifies
            // the module more clearly.
            Some(self.print_def_path(parent_def_id, None, ns, iter::empty()))
        } else {
            // Otherwise, try to give a good form that would be valid language
            // syntax. Preferably using associated item notation.
            None
        };

        self.path_qualified(prefix_path, self_ty, impl_trait_ref, ns)
    }
}

/// As a heuristic, when we see an impl, if we see that the
/// 'self type' is a type defined in the same module as the impl,
/// we can omit including the path to the impl itself. This
/// function tries to find a "characteristic `DefId`" for a
/// type. It's just a heuristic so it makes some questionable
/// decisions and we may want to adjust it later.
pub fn characteristic_def_id_of_type(ty: Ty<'_>) -> Option<DefId> {
    match ty.sty {
        ty::Adt(adt_def, _) => Some(adt_def.did),

        ty::Dynamic(data, ..) => data.principal_def_id(),

        ty::Array(subty, _) |
        ty::Slice(subty) => characteristic_def_id_of_type(subty),

        ty::RawPtr(mt) => characteristic_def_id_of_type(mt.ty),

        ty::Ref(_, ty, _) => characteristic_def_id_of_type(ty),

        ty::Tuple(ref tys) => tys.iter()
                                   .filter_map(|ty| characteristic_def_id_of_type(ty))
                                   .next(),

        ty::FnDef(def_id, _) |
        ty::Closure(def_id, _) |
        ty::Generator(def_id, _, _) |
        ty::Foreign(def_id) => Some(def_id),

        ty::Bool |
        ty::Char |
        ty::Int(_) |
        ty::Uint(_) |
        ty::Str |
        ty::FnPtr(_) |
        ty::Projection(_) |
        ty::Placeholder(..) |
        ty::UnnormalizedProjection(..) |
        ty::Param(_) |
        ty::Opaque(..) |
        ty::Infer(_) |
        ty::Bound(..) |
        ty::Error |
        ty::GeneratorWitness(..) |
        ty::Never |
        ty::Float(_) => None,
    }
}

pub struct FmtPrinter<F: fmt::Write> {
    pub fmt: F,
}

impl<P: PrettyPrinter> PrintCx<'a, 'gcx, 'tcx, P> {
    /// If possible, this returns a global path resolving to `def_id` that is visible
    /// from at least one local module and returns true. If the crate defining `def_id` is
    /// declared with an `extern crate`, the path is guaranteed to use the `extern crate`.
    fn try_print_visible_def_path(&mut self, def_id: DefId) -> Option<P::Path> {
        debug!("try_print_visible_def_path: def_id={:?}", def_id);

        // If `def_id` is a direct or injected extern crate, return the
        // path to the crate followed by the path to the item within the crate.
        if def_id.index == CRATE_DEF_INDEX {
            let cnum = def_id.krate;

            if cnum == LOCAL_CRATE {
                return Some(self.path_crate(cnum));
            }

            // In local mode, when we encounter a crate other than
            // LOCAL_CRATE, execution proceeds in one of two ways:
            //
            // 1. for a direct dependency, where user added an
            //    `extern crate` manually, we put the `extern
            //    crate` as the parent. So you wind up with
            //    something relative to the current crate.
            // 2. for an extern inferred from a path or an indirect crate,
            //    where there is no explicit `extern crate`, we just prepend
            //    the crate name.
            match *self.tcx.extern_crate(def_id) {
                Some(ExternCrate {
                    src: ExternCrateSource::Extern(def_id),
                    direct: true,
                    span,
                    ..
                }) => {
                    debug!("try_print_visible_def_path: def_id={:?}", def_id);
                    let path = if !span.is_dummy() {
                        self.print_def_path(def_id, None, Namespace::TypeNS, iter::empty())
                    } else {
                        self.path_crate(cnum)
                    };
                    return Some(path);
                }
                None => {
                    return Some(self.path_crate(cnum));
                }
                _ => {},
            }
        }

        if def_id.is_local() {
            return None;
        }

        let visible_parent_map = self.tcx.visible_parent_map(LOCAL_CRATE);

        let mut cur_def_key = self.tcx.def_key(def_id);
        debug!("try_print_visible_def_path: cur_def_key={:?}", cur_def_key);

        // For a UnitStruct or TupleStruct we want the name of its parent rather than <unnamed>.
        if let DefPathData::StructCtor = cur_def_key.disambiguated_data.data {
            let parent = DefId {
                krate: def_id.krate,
                index: cur_def_key.parent.expect("DefPathData::StructCtor missing a parent"),
            };

            cur_def_key = self.tcx.def_key(parent);
        }

        let visible_parent = visible_parent_map.get(&def_id).cloned()?;
        let path = self.try_print_visible_def_path(visible_parent)?;
        let actual_parent = self.tcx.parent(def_id);

        let data = cur_def_key.disambiguated_data.data;
        debug!(
            "try_print_visible_def_path: data={:?} visible_parent={:?} actual_parent={:?}",
            data, visible_parent, actual_parent,
        );

        let symbol = match data {
            // In order to output a path that could actually be imported (valid and visible),
            // we need to handle re-exports correctly.
            //
            // For example, take `std::os::unix::process::CommandExt`, this trait is actually
            // defined at `std::sys::unix::ext::process::CommandExt` (at time of writing).
            //
            // `std::os::unix` rexports the contents of `std::sys::unix::ext`. `std::sys` is
            // private so the "true" path to `CommandExt` isn't accessible.
            //
            // In this case, the `visible_parent_map` will look something like this:
            //
            // (child) -> (parent)
            // `std::sys::unix::ext::process::CommandExt` -> `std::sys::unix::ext::process`
            // `std::sys::unix::ext::process` -> `std::sys::unix::ext`
            // `std::sys::unix::ext` -> `std::os`
            //
            // This is correct, as the visible parent of `std::sys::unix::ext` is in fact
            // `std::os`.
            //
            // When printing the path to `CommandExt` and looking at the `cur_def_key` that
            // corresponds to `std::sys::unix::ext`, we would normally print `ext` and then go
            // to the parent - resulting in a mangled path like
            // `std::os::ext::process::CommandExt`.
            //
            // Instead, we must detect that there was a re-export and instead print `unix`
            // (which is the name `std::sys::unix::ext` was re-exported as in `std::os`). To
            // do this, we compare the parent of `std::sys::unix::ext` (`std::sys::unix`) with
            // the visible parent (`std::os`). If these do not match, then we iterate over
            // the children of the visible parent (as was done when computing
            // `visible_parent_map`), looking for the specific child we currently have and then
            // have access to the re-exported name.
            DefPathData::Module(actual_name) |
            DefPathData::TypeNs(actual_name) if Some(visible_parent) != actual_parent => {
                self.tcx.item_children(visible_parent)
                    .iter()
                    .find(|child| child.def.def_id() == def_id)
                    .map(|child| child.ident.as_str())
                    .unwrap_or_else(|| actual_name.as_str())
            }
            _ => {
                data.get_opt_name().map(|n| n.as_str()).unwrap_or_else(|| {
                    // Re-exported `extern crate` (#43189).
                    if let DefPathData::CrateRoot = data {
                        self.tcx.original_crate_name(def_id.krate).as_str()
                    } else {
                        Symbol::intern("<unnamed>").as_str()
                    }
                })
            },
        };
        debug!("try_print_visible_def_path: symbol={:?}", symbol);
        Some(self.path_append(path, &symbol))
    }

    pub fn pretty_path_qualified(
        &mut self,
        impl_prefix: Option<P::Path>,
        self_ty: Ty<'tcx>,
        trait_ref: Option<ty::TraitRef<'tcx>>,
        ns: Namespace,
    ) -> P::Path {
        if let Some(prefix) = impl_prefix {
            // HACK(eddyb) going through `path_append` means symbol name
            // computation gets to handle its equivalent of `::` correctly.
            let _ = self.path_append(prefix, "<impl ")?;
            if let Some(trait_ref) = trait_ref {
                trait_ref.print_display(self)?;
                write!(self.printer, " for ")?;
            }
            self_ty.print_display(self)?;
            write!(self.printer, ">")?;
            return Ok(PrettyPath { empty: false });
        }

        if trait_ref.is_none() {
            // Inherent impls. Try to print `Foo::bar` for an inherent
            // impl on `Foo`, but fallback to `<Foo>::bar` if self-type is
            // anything other than a simple path.
            match self_ty.sty {
                ty::Adt(adt_def, substs) => {
                    return self.print_def_path(adt_def.did, Some(substs), ns, iter::empty());
                }
                ty::Foreign(did) => {
                    return self.print_def_path(did, None, ns, iter::empty());
                }

                ty::Bool | ty::Char | ty::Str |
                ty::Int(_) | ty::Uint(_) | ty::Float(_) => {
                    self_ty.print_display(self)?;
                    return Ok(PrettyPath { empty: false });
                }

                _ => {}
            }
        }

        write!(self.printer, "<")?;
        self_ty.print_display(self)?;
        if let Some(trait_ref) = trait_ref {
            write!(self.printer, " as ")?;
            let _ = self.print_def_path(
                trait_ref.def_id,
                Some(trait_ref.substs),
                Namespace::TypeNS,
                iter::empty(),
            )?;
        }
        write!(self.printer, ">")?;
        Ok(PrettyPath { empty: false })
    }

    pub fn pretty_path_generic_args(
        &mut self,
        path: P::Path,
        params: &[ty::GenericParamDef],
        substs: SubstsRef<'tcx>,
        ns: Namespace,
        projections: impl Iterator<Item = ty::ExistentialProjection<'tcx>>,
    ) -> P::Path {
        let path = path?;

        let mut empty = true;
        let mut start_or_continue = |cx: &mut Self, start: &str, cont: &str| {
            if empty {
                empty = false;
                write!(cx.printer, "{}", start)
            } else {
                write!(cx.printer, "{}", cont)
            }
        };

        let start = if ns == Namespace::ValueNS { "::<" } else { "<" };

        // Don't print any regions if they're all erased.
        let print_regions = params.iter().any(|param| {
            match substs[param.index as usize].unpack() {
                UnpackedKind::Lifetime(r) => *r != ty::ReErased,
                _ => false,
            }
        });

        // Don't print args that are the defaults of their respective parameters.
        let num_supplied_defaults = if self.is_verbose {
            0
        } else {
            params.iter().rev().take_while(|param| {
                match param.kind {
                    ty::GenericParamDefKind::Lifetime => false,
                    ty::GenericParamDefKind::Type { has_default, .. } => {
                        has_default && substs[param.index as usize] == Kind::from(
                            self.tcx.type_of(param.def_id).subst(self.tcx, substs)
                        )
                    }
                    ty::GenericParamDefKind::Const => false, // FIXME(const_generics:defaults)
                }
            }).count()
        };

        for param in &params[..params.len() - num_supplied_defaults] {
            match substs[param.index as usize].unpack() {
                UnpackedKind::Lifetime(region) => {
                    if !print_regions {
                        continue;
                    }
                    start_or_continue(self, start, ", ")?;
                    if !region.display_outputs_anything(self) {
                        // This happens when the value of the region
                        // parameter is not easily serialized. This may be
                        // because the user omitted it in the first place,
                        // or because it refers to some block in the code,
                        // etc. I'm not sure how best to serialize this.
                        write!(self.printer, "'_")?;
                    } else {
                        region.print_display(self)?;
                    }
                }
                UnpackedKind::Type(ty) => {
                    start_or_continue(self, start, ", ")?;
                    ty.print_display(self)?;
                }
                UnpackedKind::Const(ct) => {
                    start_or_continue(self, start, ", ")?;
                    ct.print_display(self)?;
                }
            }
        }

        for projection in projections {
            start_or_continue(self, start, ", ")?;
            write!(self.printer, "{}=",
                   self.tcx.associated_item(projection.item_def_id).ident)?;
            projection.ty.print_display(self)?;
        }

        start_or_continue(self, "", ">")?;

        Ok(path)
    }
}

impl<F: fmt::Write> fmt::Write for FmtPrinter<F> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.fmt.write_str(s)
    }
}

impl<F: fmt::Write> Printer for FmtPrinter<F> {
    type Path = Result<PrettyPath, fmt::Error>;

    fn print_def_path(
        self: &mut PrintCx<'_, '_, 'tcx, Self>,
        def_id: DefId,
        substs: Option<SubstsRef<'tcx>>,
        ns: Namespace,
        projections: impl Iterator<Item = ty::ExistentialProjection<'tcx>>,
    ) -> Self::Path {
        // FIXME(eddyb) avoid querying `tcx.generics_of` and `tcx.def_key`
        // both here and in `default_print_def_path`.
        let generics = substs.map(|_| self.tcx.generics_of(def_id));
        if // HACK(eddyb) remove the `FORCE_ABSOLUTE` hack by bypassing `FmtPrinter`
            !FORCE_ABSOLUTE.with(|force| force.get()) &&
            generics.as_ref().and_then(|g| g.parent).is_none() {
            if let Some(path) = self.try_print_visible_def_path(def_id) {
                let path = if let (Some(generics), Some(substs)) = (generics, substs) {
                    let has_own_self = generics.has_self && generics.parent_count == 0;
                    let params = &generics.params[has_own_self as usize..];
                    self.path_generic_args(path, params, substs, ns, projections)
                } else {
                    path
                };
                return path;
            }
        }

        let key = self.tcx.def_key(def_id);
        if let DefPathData::Impl = key.disambiguated_data.data {
            // Always use types for non-local impls, where types are always
            // available, and filename/line-number is mostly uninteresting.
            let use_types =
                // HACK(eddyb) remove the `FORCE_ABSOLUTE` hack by bypassing `FmtPrinter`
                FORCE_ABSOLUTE.with(|force| force.get()) ||
                !def_id.is_local() || {
                    // Otherwise, use filename/line-number if forced.
                    let force_no_types = FORCE_IMPL_FILENAME_LINE.with(|f| f.get());
                    !force_no_types
                };

            if !use_types {
                // If no type info is available, fall back to
                // pretty printing some span information. This should
                // only occur very early in the compiler pipeline.
                let parent_def_id = DefId { index: key.parent.unwrap(), ..def_id };
                let path = self.print_def_path(parent_def_id, None, ns, iter::empty());
                let span = self.tcx.def_span(def_id);
                return self.path_append(path, &format!("<impl at {:?}>", span));
            }
        }

        self.default_print_def_path(def_id, substs, ns, projections)
    }

    fn path_crate(self: &mut PrintCx<'_, '_, '_, Self>, cnum: CrateNum) -> Self::Path {
        // HACK(eddyb) remove the `FORCE_ABSOLUTE` hack by bypassing `FmtPrinter`
        if FORCE_ABSOLUTE.with(|force| force.get()) {
            write!(self.printer, "{}", self.tcx.original_crate_name(cnum))?;
            return Ok(PrettyPath { empty: false });
        }
        if cnum == LOCAL_CRATE {
            if self.tcx.sess.rust_2018() {
                // We add the `crate::` keyword on Rust 2018, only when desired.
                if SHOULD_PREFIX_WITH_CRATE.with(|flag| flag.get()) {
                    write!(self.printer, "{}", keywords::Crate.name())?;
                    return Ok(PrettyPath { empty: false });
                }
            }
            Ok(PrettyPath { empty: true })
        } else {
            write!(self.printer, "{}", self.tcx.crate_name(cnum))?;
            Ok(PrettyPath { empty: false })
        }
    }
    fn path_qualified(
        self: &mut PrintCx<'_, '_, 'tcx, Self>,
        impl_prefix: Option<Self::Path>,
        self_ty: Ty<'tcx>,
        trait_ref: Option<ty::TraitRef<'tcx>>,
        ns: Namespace,
    ) -> Self::Path {
        self.pretty_path_qualified(impl_prefix, self_ty, trait_ref, ns)
    }
    fn path_append(
        self: &mut PrintCx<'_, '_, '_, Self>,
        path: Self::Path,
        text: &str,
    ) -> Self::Path {
        let path = path?;

        // FIXME(eddyb) this shouldn't happen, but is currently
        // the case for `extern { ... }` "foreign modules".
        if text.is_empty() {
            return Ok(path);
        }

        if !path.empty {
            write!(self.printer, "::")?;
        }
        write!(self.printer, "{}", text)?;
        Ok(PrettyPath { empty: false })
    }
    fn path_generic_args(
        self: &mut PrintCx<'_, '_, 'tcx, Self>,
        path: Self::Path,
        params: &[ty::GenericParamDef],
        substs: SubstsRef<'tcx>,
        ns: Namespace,
        projections: impl Iterator<Item = ty::ExistentialProjection<'tcx>>,
    ) -> Self::Path {
        self.pretty_path_generic_args(path, params, substs, ns, projections)
    }
}

impl<F: fmt::Write> PrettyPrinter for FmtPrinter<F> {}
