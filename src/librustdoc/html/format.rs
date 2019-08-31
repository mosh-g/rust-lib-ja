//! HTML formatting module
//!
//! This module contains a large number of `fmt::Display` implementations for
//! various types in `rustdoc::clean`. These implementations all currently
//! assume that HTML output is desired, although it may be possible to redesign
//! them in the future to instead emit any format desired.

use std::borrow::Cow;
use std::cell::Cell;
use std::fmt;

use rustc::hir::def_id::DefId;
use rustc::util::nodemap::FxHashSet;
use rustc_target::spec::abi::Abi;
use rustc::hir;

use crate::clean::{self, PrimitiveType};
use crate::html::item_type::ItemType;
use crate::html::render::{self, cache, CURRENT_DEPTH};

pub trait Print {
    fn print(&self, buffer: &mut Buffer);
}

impl<T: ?Sized + Print> Print for &'_ T {
    fn print(&self, buffer: &mut Buffer) {
        (&**self).print(buffer)
    }
}

impl Print for String {
    fn print(&self, buffer: &mut Buffer) {
        buffer.write_str(self);
    }
}

impl Print for str {
    fn print(&self, buffer: &mut Buffer) {
        buffer.write_str(self);
    }
}

#[derive(Debug, Clone)]
pub struct Buffer {
    for_html: bool,
    buffer: String,
}

impl Buffer {
    crate fn empty_from(v: &Buffer) -> Buffer {
        Buffer {
            for_html: v.for_html,
            buffer: String::new(),
        }
    }

    crate fn html() -> Buffer {
        Buffer {
            for_html: true,
            buffer: String::new(),
        }
    }

    crate fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    crate fn into_inner(self) -> String {
        self.buffer
    }

    crate fn insert_str(&mut self, idx: usize, s: &str) {
        self.buffer.insert_str(idx, s);
    }

    crate fn push_str(&mut self, s: &str) {
        self.buffer.push_str(s);
    }

    // Intended for consumption by write! and writeln! (std::fmt) but without
    // the fmt::Result return type imposed by fmt::Write (and avoiding the trait
    // import).
    crate fn write_str(&mut self, s: &str) {
        self.buffer.push_str(s);
    }

    // Intended for consumption by write! and writeln! (std::fmt) but without
    // the fmt::Result return type imposed by fmt::Write (and avoiding the trait
    // import).
    crate fn write_fmt(&mut self, v: fmt::Arguments<'_>) {
        use fmt::Write;
        self.buffer.write_fmt(v).unwrap();
    }

    crate fn to_display<T: ?Sized + Print>(mut self, t: &T) -> String {
        t.print(&mut self);
        self.into_inner()
    }

    crate fn display<T: fmt::Display>(&mut self, t: T) {
        if self.for_html {
            write!(self, "{}", t);
        } else {
            write!(self, "{:#}", t);
        }
    }
}

/// Helper to render an optional visibility with a space after it (if the
/// visibility is preset)
#[derive(Copy, Clone)]
pub struct VisSpace<'a>(pub &'a Option<clean::Visibility>);
/// Similarly to VisSpace, this structure is used to render a function style with a
/// space after it.
#[derive(Copy, Clone)]
pub struct UnsafetySpace(pub hir::Unsafety);
/// Similarly to VisSpace, this structure is used to render a function constness
/// with a space after it.
#[derive(Copy, Clone)]
pub struct ConstnessSpace(pub hir::Constness);
/// Similarly to VisSpace, this structure is used to render a function asyncness
/// with a space after it.
#[derive(Copy, Clone)]
pub struct AsyncSpace(pub hir::IsAsync);
/// Similar to VisSpace, but used for mutability
#[derive(Copy, Clone)]
pub struct MutableSpace(pub clean::Mutability);
/// Wrapper struct for emitting type parameter bounds.
pub struct GenericBounds<'a>(pub &'a [clean::GenericBound]);
pub struct AbiSpace(pub Abi);
pub struct DefaultSpace(pub bool);

/// Wrapper struct for properly emitting a function or method declaration.
pub struct Function<'a> {
    /// The declaration to emit.
    pub decl: &'a clean::FnDecl,
    /// The length of the function header and name. In other words, the number of characters in the
    /// function declaration up to but not including the parentheses.
    ///
    /// Used to determine line-wrapping.
    pub header_len: usize,
    /// The number of spaces to indent each successive line with, if line-wrapping is necessary.
    pub indent: usize,
    /// Whether the function is async or not.
    pub asyncness: hir::IsAsync,
}

/// Wrapper struct for emitting a where-clause from Generics.
pub struct WhereClause<'a>{
    /// The Generics from which to emit a where-clause.
    pub gens: &'a clean::Generics,
    /// The number of spaces to indent each line with.
    pub indent: usize,
    /// Whether the where-clause needs to add a comma and newline after the last bound.
    pub end_newline: bool,
}

impl<'a> VisSpace<'a> {
    pub fn get(self) -> &'a Option<clean::Visibility> {
        let VisSpace(v) = self; v
    }
}

impl UnsafetySpace {
    pub fn get(&self) -> hir::Unsafety {
        let UnsafetySpace(v) = *self; v
    }
}

impl ConstnessSpace {
    pub fn get(&self) -> hir::Constness {
        let ConstnessSpace(v) = *self; v
    }
}

fn comma_sep<T: fmt::Display>(items: &[T]) -> impl fmt::Display + '_ {
    display_fn(move |f| {
        for (i, item) in items.iter().enumerate() {
            if i != 0 { write!(f, ", ")?; }
            fmt::Display::fmt(item, f)?;
        }
        Ok(())
    })
}

impl<'a> fmt::Display for GenericBounds<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut bounds_dup = FxHashSet::default();
        let &GenericBounds(bounds) = self;

        for (i, bound) in bounds.iter().filter(|b| bounds_dup.insert(b.to_string())).enumerate() {
            if i > 0 {
                f.write_str(" + ")?;
            }
            fmt::Display::fmt(bound, f)?;
        }
        Ok(())
    }
}

impl fmt::Display for clean::GenericParamDef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            clean::GenericParamDefKind::Lifetime => write!(f, "{}", self.name),
            clean::GenericParamDefKind::Type { ref bounds, ref default, .. } => {
                f.write_str(&self.name)?;

                if !bounds.is_empty() {
                    if f.alternate() {
                        write!(f, ": {:#}", GenericBounds(bounds))?;
                    } else {
                        write!(f, ":&nbsp;{}", GenericBounds(bounds))?;
                    }
                }

                if let Some(ref ty) = default {
                    if f.alternate() {
                        write!(f, " = {:#}", ty)?;
                    } else {
                        write!(f, "&nbsp;=&nbsp;{}", ty)?;
                    }
                }

                Ok(())
            }
            clean::GenericParamDefKind::Const { ref ty, .. } => {
                f.write_str("const ")?;
                f.write_str(&self.name)?;

                if f.alternate() {
                    write!(f, ": {:#}", ty)
                } else {
                    write!(f, ":&nbsp;{}", ty)
                }
            }
        }
    }
}

impl fmt::Display for clean::Generics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let real_params = self.params
            .iter()
            .filter(|p| !p.is_synthetic_type_param())
            .collect::<Vec<_>>();
        if real_params.is_empty() {
            return Ok(());
        }
        if f.alternate() {
            write!(f, "<{:#}>", comma_sep(&real_params))
        } else {
            write!(f, "&lt;{}&gt;", comma_sep(&real_params))
        }
    }
}

impl<'a> fmt::Display for WhereClause<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let &WhereClause { gens, indent, end_newline } = self;
        if gens.where_predicates.is_empty() {
            return Ok(());
        }
        let mut clause = String::new();
        if f.alternate() {
            clause.push_str(" where");
        } else {
            if end_newline {
                clause.push_str(" <span class=\"where fmt-newline\">where");
            } else {
                clause.push_str(" <span class=\"where\">where");
            }
        }
        for (i, pred) in gens.where_predicates.iter().enumerate() {
            if f.alternate() {
                clause.push(' ');
            } else {
                clause.push_str("<br>");
            }

            match pred {
                &clean::WherePredicate::BoundPredicate { ref ty, ref bounds } => {
                    let bounds = bounds;
                    if f.alternate() {
                        clause.push_str(&format!("{:#}: {:#}", ty, GenericBounds(bounds)));
                    } else {
                        clause.push_str(&format!("{}: {}", ty, GenericBounds(bounds)));
                    }
                }
                &clean::WherePredicate::RegionPredicate { ref lifetime, ref bounds } => {
                    clause.push_str(&format!("{}: {}",
                                             lifetime,
                                             bounds.iter()
                                                   .map(|b| b.to_string())
                                                   .collect::<Vec<_>>()
                                                   .join(" + ")));
                }
                &clean::WherePredicate::EqPredicate { ref lhs, ref rhs } => {
                    if f.alternate() {
                        clause.push_str(&format!("{:#} == {:#}", lhs, rhs));
                    } else {
                        clause.push_str(&format!("{} == {}", lhs, rhs));
                    }
                }
            }

            if i < gens.where_predicates.len() - 1 || end_newline {
                clause.push(',');
            }
        }

        if end_newline {
            // add a space so stripping <br> tags and breaking spaces still renders properly
            if f.alternate() {
                clause.push(' ');
            } else {
                clause.push_str("&nbsp;");
            }
        }

        if !f.alternate() {
            clause.push_str("</span>");
            let padding = "&nbsp;".repeat(indent + 4);
            clause = clause.replace("<br>", &format!("<br>{}", padding));
            clause.insert_str(0, &"&nbsp;".repeat(indent.saturating_sub(1)));
            if !end_newline {
                clause.insert_str(0, "<br>");
            }
        }
        write!(f, "{}", clause)
    }
}

impl fmt::Display for clean::Lifetime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.get_ref())?;
        Ok(())
    }
}

impl fmt::Display for clean::Constant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.expr, f)
    }
}

impl fmt::Display for clean::PolyTrait {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.generic_params.is_empty() {
            if f.alternate() {
                write!(f, "for<{:#}> ", comma_sep(&self.generic_params))?;
            } else {
                write!(f, "for&lt;{}&gt; ", comma_sep(&self.generic_params))?;
            }
        }
        if f.alternate() {
            write!(f, "{:#}", self.trait_)
        } else {
            write!(f, "{}", self.trait_)
        }
    }
}

impl fmt::Display for clean::GenericBound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            clean::GenericBound::Outlives(ref lt) => {
                write!(f, "{}", *lt)
            }
            clean::GenericBound::TraitBound(ref ty, modifier) => {
                let modifier_str = match modifier {
                    hir::TraitBoundModifier::None => "",
                    hir::TraitBoundModifier::Maybe => "?",
                };
                if f.alternate() {
                    write!(f, "{}{:#}", modifier_str, *ty)
                } else {
                    write!(f, "{}{}", modifier_str, *ty)
                }
            }
        }
    }
}

impl fmt::Display for clean::GenericArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            clean::GenericArgs::AngleBracketed { ref args, ref bindings } => {
                if !args.is_empty() || !bindings.is_empty() {
                    if f.alternate() {
                        f.write_str("<")?;
                    } else {
                        f.write_str("&lt;")?;
                    }
                    let mut comma = false;
                    for arg in args {
                        if comma {
                            f.write_str(", ")?;
                        }
                        comma = true;
                        if f.alternate() {
                            write!(f, "{:#}", *arg)?;
                        } else {
                            write!(f, "{}", *arg)?;
                        }
                    }
                    for binding in bindings {
                        if comma {
                            f.write_str(", ")?;
                        }
                        comma = true;
                        if f.alternate() {
                            write!(f, "{:#}", *binding)?;
                        } else {
                            write!(f, "{}", *binding)?;
                        }
                    }
                    if f.alternate() {
                        f.write_str(">")?;
                    } else {
                        f.write_str("&gt;")?;
                    }
                }
            }
            clean::GenericArgs::Parenthesized { ref inputs, ref output } => {
                f.write_str("(")?;
                let mut comma = false;
                for ty in inputs {
                    if comma {
                        f.write_str(", ")?;
                    }
                    comma = true;
                    if f.alternate() {
                        write!(f, "{:#}", *ty)?;
                    } else {
                        write!(f, "{}", *ty)?;
                    }
                }
                f.write_str(")")?;
                if let Some(ref ty) = *output {
                    if f.alternate() {
                        write!(f, " -> {:#}", ty)?;
                    } else {
                        write!(f, " -&gt; {}", ty)?;
                    }
                }
            }
        }
        Ok(())
    }
}

impl fmt::Display for clean::PathSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)?;
        if f.alternate() {
            write!(f, "{:#}", self.args)
        } else {
            write!(f, "{}", self.args)
        }
    }
}

impl fmt::Display for clean::Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.global {
            f.write_str("::")?
        }

        for (i, seg) in self.segments.iter().enumerate() {
            if i > 0 {
                f.write_str("::")?
            }
            if f.alternate() {
                write!(f, "{:#}", seg)?;
            } else {
                write!(f, "{}", seg)?;
            }
        }
        Ok(())
    }
}

pub fn href(did: DefId) -> Option<(String, ItemType, Vec<String>)> {
    let cache = cache();
    if !did.is_local() && !cache.access_levels.is_public(did) {
        return None
    }

    let depth = CURRENT_DEPTH.with(|l| l.get());
    let (fqp, shortty, mut url) = match cache.paths.get(&did) {
        Some(&(ref fqp, shortty)) => {
            (fqp, shortty, "../".repeat(depth))
        }
        None => {
            let &(ref fqp, shortty) = cache.external_paths.get(&did)?;
            (fqp, shortty, match cache.extern_locations[&did.krate] {
                (.., render::Remote(ref s)) => s.to_string(),
                (.., render::Local) => "../".repeat(depth),
                (.., render::Unknown) => return None,
            })
        }
    };
    for component in &fqp[..fqp.len() - 1] {
        url.push_str(component);
        url.push_str("/");
    }
    match shortty {
        ItemType::Module => {
            url.push_str(fqp.last().unwrap());
            url.push_str("/index.html");
        }
        _ => {
            url.push_str(shortty.css_class());
            url.push_str(".");
            url.push_str(fqp.last().unwrap());
            url.push_str(".html");
        }
    }
    Some((url, shortty, fqp.to_vec()))
}

/// Used when rendering a `ResolvedPath` structure. This invokes the `path`
/// rendering function with the necessary arguments for linking to a local path.
fn resolved_path(w: &mut fmt::Formatter<'_>, did: DefId, path: &clean::Path,
                 print_all: bool, use_absolute: bool) -> fmt::Result {
    let last = path.segments.last().unwrap();

    if print_all {
        for seg in &path.segments[..path.segments.len() - 1] {
            write!(w, "{}::", seg.name)?;
        }
    }
    if w.alternate() {
        write!(w, "{}{:#}", &last.name, last.args)?;
    } else {
        let path = if use_absolute {
            if let Some((_, _, fqp)) = href(did) {
                format!("{}::{}",
                        fqp[..fqp.len() - 1].join("::"),
                        anchor(did, fqp.last().unwrap()))
            } else {
                last.name.to_string()
            }
        } else {
            anchor(did, &last.name).to_string()
        };
        write!(w, "{}{}", path, last.args)?;
    }
    Ok(())
}

fn primitive_link(f: &mut fmt::Formatter<'_>,
                  prim: clean::PrimitiveType,
                  name: &str) -> fmt::Result {
    let m = cache();
    let mut needs_termination = false;
    if !f.alternate() {
        match m.primitive_locations.get(&prim) {
            Some(&def_id) if def_id.is_local() => {
                let len = CURRENT_DEPTH.with(|s| s.get());
                let len = if len == 0 {0} else {len - 1};
                write!(f, "<a class=\"primitive\" href=\"{}primitive.{}.html\">",
                       "../".repeat(len),
                       prim.to_url_str())?;
                needs_termination = true;
            }
            Some(&def_id) => {
                let loc = match m.extern_locations[&def_id.krate] {
                    (ref cname, _, render::Remote(ref s)) => {
                        Some((cname, s.to_string()))
                    }
                    (ref cname, _, render::Local) => {
                        let len = CURRENT_DEPTH.with(|s| s.get());
                        Some((cname, "../".repeat(len)))
                    }
                    (.., render::Unknown) => None,
                };
                if let Some((cname, root)) = loc {
                    write!(f, "<a class=\"primitive\" href=\"{}{}/primitive.{}.html\">",
                           root,
                           cname,
                           prim.to_url_str())?;
                    needs_termination = true;
                }
            }
            None => {}
        }
    }
    write!(f, "{}", name)?;
    if needs_termination {
        write!(f, "</a>")?;
    }
    Ok(())
}

/// Helper to render type parameters
fn tybounds(param_names: &Option<Vec<clean::GenericBound>>) -> impl fmt::Display + '_ {
    display_fn(move |f| {
        match *param_names {
            Some(ref params) => {
                for param in params {
                    write!(f, " + ")?;
                    fmt::Display::fmt(param, f)?;
                }
                Ok(())
            }
            None => Ok(())
        }
    })
}

pub fn anchor(did: DefId, text: &str) -> impl fmt::Display + '_ {
    display_fn(move |f| {
        if let Some((url, short_ty, fqp)) = href(did) {
            write!(f, r#"<a class="{}" href="{}" title="{} {}">{}</a>"#,
                short_ty, url, short_ty, fqp.join("::"), text)
        } else {
            write!(f, "{}", text)
        }
    })
}

fn fmt_type(t: &clean::Type, f: &mut fmt::Formatter<'_>, use_absolute: bool) -> fmt::Result {
    match *t {
        clean::Generic(ref name) => {
            f.write_str(name)
        }
        clean::ResolvedPath{ did, ref param_names, ref path, is_generic } => {
            if param_names.is_some() {
                f.write_str("dyn ")?;
            }
            // Paths like `T::Output` and `Self::Output` should be rendered with all segments.
            resolved_path(f, did, path, is_generic, use_absolute)?;
            fmt::Display::fmt(&tybounds(param_names), f)
        }
        clean::Infer => write!(f, "_"),
        clean::Primitive(prim) => primitive_link(f, prim, prim.as_str()),
        clean::BareFunction(ref decl) => {
            if f.alternate() {
                write!(f, "{}{:#}fn{:#}{:#}",
                       UnsafetySpace(decl.unsafety),
                       AbiSpace(decl.abi),
                       comma_sep(&decl.generic_params),
                       decl.decl)
            } else {
                write!(f, "{}{}", UnsafetySpace(decl.unsafety), AbiSpace(decl.abi))?;
                primitive_link(f, PrimitiveType::Fn, "fn")?;
                write!(f, "{}{}", comma_sep(&decl.generic_params), decl.decl)
            }
        }
        clean::Tuple(ref typs) => {
            match &typs[..] {
                &[] => primitive_link(f, PrimitiveType::Unit, "()"),
                &[ref one] => {
                    primitive_link(f, PrimitiveType::Tuple, "(")?;
                    // Carry `f.alternate()` into this display w/o branching manually.
                    fmt::Display::fmt(one, f)?;
                    primitive_link(f, PrimitiveType::Tuple, ",)")
                }
                many => {
                    primitive_link(f, PrimitiveType::Tuple, "(")?;
                    fmt::Display::fmt(&comma_sep(many), f)?;
                    primitive_link(f, PrimitiveType::Tuple, ")")
                }
            }
        }
        clean::Slice(ref t) => {
            primitive_link(f, PrimitiveType::Slice, "[")?;
            fmt::Display::fmt(t, f)?;
            primitive_link(f, PrimitiveType::Slice, "]")
        }
        clean::Array(ref t, ref n) => {
            primitive_link(f, PrimitiveType::Array, "[")?;
            fmt::Display::fmt(t, f)?;
            primitive_link(f, PrimitiveType::Array, &format!("; {}]", n))
        }
        clean::Never => primitive_link(f, PrimitiveType::Never, "!"),
        clean::CVarArgs => primitive_link(f, PrimitiveType::CVarArgs, "..."),
        clean::RawPointer(m, ref t) => {
            let m = match m {
                clean::Immutable => "const",
                clean::Mutable => "mut",
            };
            match **t {
                clean::Generic(_) | clean::ResolvedPath {is_generic: true, ..} => {
                    if f.alternate() {
                        primitive_link(f, clean::PrimitiveType::RawPointer,
                                       &format!("*{} {:#}", m, t))
                    } else {
                        primitive_link(f, clean::PrimitiveType::RawPointer,
                                       &format!("*{} {}", m, t))
                    }
                }
                _ => {
                    primitive_link(f, clean::PrimitiveType::RawPointer, &format!("*{} ", m))?;
                    fmt::Display::fmt(t, f)
                }
            }
        }
        clean::BorrowedRef{ lifetime: ref l, mutability, type_: ref ty} => {
            let lt = match *l {
                Some(ref l) => format!("{} ", *l),
                _ => String::new(),
            };
            let m = MutableSpace(mutability);
            let amp = if f.alternate() {
                "&".to_string()
            } else {
                "&amp;".to_string()
            };
            match **ty {
                clean::Slice(ref bt) => { // `BorrowedRef{ ... Slice(T) }` is `&[T]`
                    match **bt {
                        clean::Generic(_) => {
                            if f.alternate() {
                                primitive_link(f, PrimitiveType::Slice,
                                    &format!("{}{}{}[{:#}]", amp, lt, m, **bt))
                            } else {
                                primitive_link(f, PrimitiveType::Slice,
                                    &format!("{}{}{}[{}]", amp, lt, m, **bt))
                            }
                        }
                        _ => {
                            primitive_link(f, PrimitiveType::Slice,
                                           &format!("{}{}{}[", amp, lt, m))?;
                            if f.alternate() {
                                write!(f, "{:#}", **bt)?;
                            } else {
                                write!(f, "{}", **bt)?;
                            }
                            primitive_link(f, PrimitiveType::Slice, "]")
                        }
                    }
                }
                clean::ResolvedPath { param_names: Some(ref v), .. } if !v.is_empty() => {
                    write!(f, "{}{}{}(", amp, lt, m)?;
                    fmt_type(&ty, f, use_absolute)?;
                    write!(f, ")")
                }
                clean::Generic(..) => {
                    primitive_link(f, PrimitiveType::Reference,
                                   &format!("{}{}{}", amp, lt, m))?;
                    fmt_type(&ty, f, use_absolute)
                }
                _ => {
                    write!(f, "{}{}{}", amp, lt, m)?;
                    fmt_type(&ty, f, use_absolute)
                }
            }
        }
        clean::ImplTrait(ref bounds) => {
            if f.alternate() {
                write!(f, "impl {:#}", GenericBounds(bounds))
            } else {
                write!(f, "impl {}", GenericBounds(bounds))
            }
        }
        clean::QPath { ref name, ref self_type, ref trait_ } => {
            let should_show_cast = match *trait_ {
                box clean::ResolvedPath { ref path, .. } => {
                    !path.segments.is_empty() && !self_type.is_self_type()
                }
                _ => true,
            };
            if f.alternate() {
                if should_show_cast {
                    write!(f, "<{:#} as {:#}>::", self_type, trait_)?
                } else {
                    write!(f, "{:#}::", self_type)?
                }
            } else {
                if should_show_cast {
                    write!(f, "&lt;{} as {}&gt;::", self_type, trait_)?
                } else {
                    write!(f, "{}::", self_type)?
                }
            };
            match *trait_ {
                // It's pretty unsightly to look at `<A as B>::C` in output, and
                // we've got hyperlinking on our side, so try to avoid longer
                // notation as much as possible by making `C` a hyperlink to trait
                // `B` to disambiguate.
                //
                // FIXME: this is still a lossy conversion and there should probably
                //        be a better way of representing this in general? Most of
                //        the ugliness comes from inlining across crates where
                //        everything comes in as a fully resolved QPath (hard to
                //        look at).
                box clean::ResolvedPath { did, ref param_names, .. } => {
                    match href(did) {
                        Some((ref url, _, ref path)) if !f.alternate() => {
                            write!(f,
                                   "<a class=\"type\" href=\"{url}#{shortty}.{name}\" \
                                   title=\"type {path}::{name}\">{name}</a>",
                                   url = url,
                                   shortty = ItemType::AssocType,
                                   name = name,
                                   path = path.join("::"))?;
                        }
                        _ => write!(f, "{}", name)?,
                    }

                    // FIXME: `param_names` are not rendered, and this seems bad?
                    drop(param_names);
                    Ok(())
                }
                _ => {
                    write!(f, "{}", name)
                }
            }
        }
    }
}

impl fmt::Display for clean::Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_type(self, f, false)
    }
}

fn fmt_impl(i: &clean::Impl,
            f: &mut fmt::Formatter<'_>,
            link_trait: bool,
            use_absolute: bool) -> fmt::Result {
    if f.alternate() {
        write!(f, "impl{:#} ", i.generics)?;
    } else {
        write!(f, "impl{} ", i.generics)?;
    }

    if let Some(ref ty) = i.trait_ {
        if i.polarity == Some(clean::ImplPolarity::Negative) {
            write!(f, "!")?;
        }

        if link_trait {
            fmt::Display::fmt(ty, f)?;
        } else {
            match *ty {
                clean::ResolvedPath { param_names: None, ref path, is_generic: false, .. } => {
                    let last = path.segments.last().unwrap();
                    fmt::Display::fmt(&last.name, f)?;
                    fmt::Display::fmt(&last.args, f)?;
                }
                _ => unreachable!(),
            }
        }
        write!(f, " for ")?;
    }

    if let Some(ref ty) = i.blanket_impl {
        fmt_type(ty, f, use_absolute)?;
    } else {
        fmt_type(&i.for_, f, use_absolute)?;
    }

    fmt::Display::fmt(&WhereClause { gens: &i.generics, indent: 0, end_newline: true }, f)?;
    Ok(())
}

impl fmt::Display for clean::Impl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_impl(self, f, true, false)
    }
}

// The difference from above is that trait is not hyperlinked.
pub fn fmt_impl_for_trait_page(i: &clean::Impl,
                               f: &mut fmt::Formatter<'_>,
                               use_absolute: bool) -> fmt::Result {
    fmt_impl(i, f, false, use_absolute)
}

impl fmt::Display for clean::Arguments {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, input) in self.values.iter().enumerate() {
            if !input.name.is_empty() {
                write!(f, "{}: ", input.name)?;
            }
            if f.alternate() {
                write!(f, "{:#}", input.type_)?;
            } else {
                write!(f, "{}", input.type_)?;
            }
            if i + 1 < self.values.len() { write!(f, ", ")?; }
        }
        Ok(())
    }
}

impl fmt::Display for clean::FunctionRetTy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            clean::Return(clean::Tuple(ref tys)) if tys.is_empty() => Ok(()),
            clean::Return(ref ty) if f.alternate() => write!(f, " -> {:#}", ty),
            clean::Return(ref ty) => write!(f, " -&gt; {}", ty),
            clean::DefaultReturn => Ok(()),
        }
    }
}

impl fmt::Display for clean::FnDecl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if f.alternate() {
            write!(f, "({args:#}){arrow:#}", args = self.inputs, arrow = self.output)
        } else {
            write!(f, "({args}){arrow}", args = self.inputs, arrow = self.output)
        }
    }
}

impl<'a> fmt::Display for Function<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let &Function { decl, header_len, indent, asyncness } = self;
        let amp = if f.alternate() { "&" } else { "&amp;" };
        let mut args = String::new();
        let mut args_plain = String::new();
        for (i, input) in decl.inputs.values.iter().enumerate() {
            if i == 0 {
                args.push_str("<br>");
            }

            if let Some(selfty) = input.to_self() {
                match selfty {
                    clean::SelfValue => {
                        args.push_str("self");
                        args_plain.push_str("self");
                    }
                    clean::SelfBorrowed(Some(ref lt), mtbl) => {
                        args.push_str(&format!("{}{} {}self", amp, *lt, MutableSpace(mtbl)));
                        args_plain.push_str(&format!("&{} {}self", *lt, MutableSpace(mtbl)));
                    }
                    clean::SelfBorrowed(None, mtbl) => {
                        args.push_str(&format!("{}{}self", amp, MutableSpace(mtbl)));
                        args_plain.push_str(&format!("&{}self", MutableSpace(mtbl)));
                    }
                    clean::SelfExplicit(ref typ) => {
                        if f.alternate() {
                            args.push_str(&format!("self: {:#}", *typ));
                        } else {
                            args.push_str(&format!("self: {}", *typ));
                        }
                        args_plain.push_str(&format!("self: {:#}", *typ));
                    }
                }
            } else {
                if i > 0 {
                    args.push_str(" <br>");
                    args_plain.push_str(" ");
                }
                if !input.name.is_empty() {
                    args.push_str(&format!("{}: ", input.name));
                    args_plain.push_str(&format!("{}: ", input.name));
                }

                if f.alternate() {
                    args.push_str(&format!("{:#}", input.type_));
                } else {
                    args.push_str(&input.type_.to_string());
                }
                args_plain.push_str(&format!("{:#}", input.type_));
            }
            if i + 1 < decl.inputs.values.len() {
                args.push(',');
                args_plain.push(',');
            }
        }

        let args_plain = format!("({})", args_plain);

        let output = if let hir::IsAsync::Async = asyncness {
            Cow::Owned(decl.sugared_async_return_type())
        } else {
            Cow::Borrowed(&decl.output)
        };

        let arrow_plain = format!("{:#}", &output);
        let arrow = if f.alternate() {
            format!("{:#}", &output)
        } else {
            output.to_string()
        };

        let declaration_len = header_len + args_plain.len() + arrow_plain.len();
        let output = if declaration_len > 80 {
            let full_pad = format!("<br>{}", "&nbsp;".repeat(indent + 4));
            let close_pad = format!("<br>{}", "&nbsp;".repeat(indent));
            format!("({args}{close}){arrow}",
                    args = args.replace("<br>", &full_pad),
                    close = close_pad,
                    arrow = arrow)
        } else {
            format!("({args}){arrow}", args = args.replace("<br>", ""), arrow = arrow)
        };

        if f.alternate() {
            write!(f, "{}", output.replace("<br>", "\n"))
        } else {
            write!(f, "{}", output)
        }
    }
}

impl<'a> fmt::Display for VisSpace<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self.get() {
            Some(clean::Public) => f.write_str("pub "),
            Some(clean::Inherited) | None => Ok(()),
            Some(clean::Visibility::Crate) => write!(f, "pub(crate) "),
            Some(clean::Visibility::Restricted(did, ref path)) => {
                f.write_str("pub(")?;
                if path.segments.len() != 1
                    || (path.segments[0].name != "self" && path.segments[0].name != "super")
                {
                    f.write_str("in ")?;
                }
                resolved_path(f, did, path, true, false)?;
                f.write_str(") ")
            }
        }
    }
}

impl fmt::Display for UnsafetySpace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.get() {
            hir::Unsafety::Unsafe => write!(f, "unsafe "),
            hir::Unsafety::Normal => Ok(())
        }
    }
}

impl fmt::Display for ConstnessSpace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.get() {
            hir::Constness::Const => write!(f, "const "),
            hir::Constness::NotConst => Ok(())
        }
    }
}

impl fmt::Display for AsyncSpace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            hir::IsAsync::Async => write!(f, "async "),
            hir::IsAsync::NotAsync => Ok(()),
        }
    }
}

impl fmt::Display for clean::Import {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            clean::Import::Simple(ref name, ref src) => {
                if *name == src.path.last_name() {
                    write!(f, "use {};", *src)
                } else {
                    write!(f, "use {} as {};", *src, *name)
                }
            }
            clean::Import::Glob(ref src) => {
                if src.path.segments.is_empty() {
                    write!(f, "use *;")
                } else {
                    write!(f, "use {}::*;", *src)
                }
            }
        }
    }
}

impl fmt::Display for clean::ImportSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.did {
            Some(did) => resolved_path(f, did, &self.path, true, false),
            _ => {
                for (i, seg) in self.path.segments.iter().enumerate() {
                    if i > 0 {
                        write!(f, "::")?
                    }
                    write!(f, "{}", seg.name)?;
                }
                Ok(())
            }
        }
    }
}

impl fmt::Display for clean::TypeBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)?;
        match self.kind {
            clean::TypeBindingKind::Equality { ref ty } => {
                if f.alternate() {
                    write!(f, " = {:#}", ty)?;
                } else {
                    write!(f, " = {}", ty)?;
                }
            }
            clean::TypeBindingKind::Constraint { ref bounds } => {
                if !bounds.is_empty() {
                    if f.alternate() {
                        write!(f, ": {:#}", GenericBounds(bounds))?;
                    } else {
                        write!(f, ":&nbsp;{}", GenericBounds(bounds))?;
                    }
                }
            }
        }
        Ok(())
    }
}

impl fmt::Display for MutableSpace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            MutableSpace(clean::Immutable) => Ok(()),
            MutableSpace(clean::Mutable) => write!(f, "mut "),
        }
    }
}

impl fmt::Display for AbiSpace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let quot = if f.alternate() { "\"" } else { "&quot;" };
        match self.0 {
            Abi::Rust => Ok(()),
            abi => write!(f, "extern {0}{1}{0} ", quot, abi.name()),
        }
    }
}

impl fmt::Display for DefaultSpace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 {
            write!(f, "default ")
        } else {
            Ok(())
        }
    }
}

crate fn display_fn(
    f: impl FnOnce(&mut fmt::Formatter<'_>) -> fmt::Result,
) -> impl fmt::Display {
    WithFormatter(Cell::new(Some(f)))
}

struct WithFormatter<F>(Cell<Option<F>>);

impl<F> fmt::Display for WithFormatter<F>
    where F: FnOnce(&mut fmt::Formatter<'_>) -> fmt::Result,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (self.0.take()).unwrap()(f)
    }
}
