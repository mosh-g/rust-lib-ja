use proc_macro::TokenStream;
use proc_macro2::Span;
use syn::{
    Token, Ident, Type, Attribute, ReturnType, Expr, Block, Error,
    braced, parenthesized, parse_macro_input,
};
use syn::spanned::Spanned;
use syn::parse::{Result, Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn;
use quote::quote;

#[allow(non_camel_case_types)]
mod kw {
    syn::custom_keyword!(query);
}

/// Ident or a wildcard `_`.
struct IdentOrWild(Ident);

impl Parse for IdentOrWild {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        Ok(if input.peek(Token![_]) {
            input.parse::<Token![_]>()?;
            IdentOrWild(Ident::new("_", Span::call_site()))
        } else {
            IdentOrWild(input.parse()?)
        })
    }
}

/// A modifier for a query
enum QueryModifier {
    /// The description of the query
    Desc(Option<Ident>, Punctuated<Expr, Token![,]>),

    /// Cache the query to disk if the `Expr` returns true.
    Cache(Option<Ident>, Expr),

    /// Custom code to load the query from disk.
    LoadCached(Ident, Ident, Block),

    /// A cycle error for this query aborting the compilation with a fatal error.
    FatalCycle,
}

impl Parse for QueryModifier {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let modifier: Ident = input.parse()?;
        if modifier == "desc" {
            // Parse a description modifier like:
            // `desc { |tcx| "foo {}", tcx.item_path(key) }`
            let attr_content;
            braced!(attr_content in input);
            let tcx = if attr_content.peek(Token![|]) {
                attr_content.parse::<Token![|]>()?;
                let tcx = attr_content.parse()?;
                attr_content.parse::<Token![|]>()?;
                Some(tcx)
            } else {
                None
            };
            let desc = attr_content.parse_terminated(Expr::parse)?;
            Ok(QueryModifier::Desc(tcx, desc))
        } else if modifier == "cache" {
            // Parse a cache modifier like:
            // `cache { |tcx| key.is_local() }`
            let attr_content;
            braced!(attr_content in input);
            let tcx = if attr_content.peek(Token![|]) {
                attr_content.parse::<Token![|]>()?;
                let tcx = attr_content.parse()?;
                attr_content.parse::<Token![|]>()?;
                Some(tcx)
            } else {
                None
            };
            let expr = attr_content.parse()?;
            Ok(QueryModifier::Cache(tcx, expr))
        } else if modifier == "load_cached" {
            // Parse a load_cached modifier like:
            // `load_cached(tcx, id) { tcx.queries.on_disk_cache.try_load_query_result(tcx, id) }`
            let args;
            parenthesized!(args in input);
            let tcx = args.parse()?;
            args.parse::<Token![,]>()?;
            let id = args.parse()?;
            let block = input.parse()?;
            Ok(QueryModifier::LoadCached(tcx, id, block))
        } else if modifier == "fatal_cycle" {
            Ok(QueryModifier::FatalCycle)
        } else {
            Err(Error::new(modifier.span(), "unknown query modifier"))
        }
    }
}

/// Ensures only doc comment attributes are used
fn check_attributes(attrs: Vec<Attribute>) -> Result<()> {
    for attr in attrs {
        if !attr.path.is_ident("doc") {
            return Err(Error::new(attr.span(), "attributes not supported on queries"));
        }
    }
    Ok(())
}

/// A compiler query. `query ... { ... }`
struct Query {
    attrs: List<QueryModifier>,
    name: Ident,
    key: IdentOrWild,
    arg: Type,
    result: ReturnType,
}

impl Parse for Query {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        check_attributes(input.call(Attribute::parse_outer)?)?;

        // Parse the query declaration. Like `query type_of(key: DefId) -> Ty<'tcx>`
        input.parse::<kw::query>()?;
        let name: Ident = input.parse()?;
        let arg_content;
        parenthesized!(arg_content in input);
        let key = arg_content.parse()?;
        arg_content.parse::<Token![:]>()?;
        let arg = arg_content.parse()?;
        let result = input.parse()?;

        // Parse the query modifiers
        let content;
        braced!(content in input);
        let attrs = content.parse()?;

        Ok(Query {
            attrs,
            name,
            key,
            arg,
            result,
        })
    }
}

/// A type used to greedily parse another type until the input is empty.
struct List<T>(Vec<T>);

impl<T: Parse> Parse for List<T> {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut list = Vec::new();
        while !input.is_empty() {
            list.push(input.parse()?);
        }
        Ok(List(list))
    }
}

/// A named group containing queries.
struct Group {
    name: Ident,
    queries: List<Query>,
}

impl Parse for Group {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let name: Ident = input.parse()?;
        let content;
        braced!(content in input);
        Ok(Group {
            name,
            queries: content.parse()?,
        })
    }
}

/// Add the impl of QueryDescription for the query to `impls` if one is requested
fn add_query_description_impl(query: &Query, impls: &mut proc_macro2::TokenStream) {
    let name = &query.name;
    let arg = &query.arg;
    let key = &query.key.0;

    // Find custom code to load the query from disk
    let load_cached = query.attrs.0.iter().find_map(|attr| match attr {
        QueryModifier::LoadCached(tcx, id, block) => Some((tcx, id, block)),
        _ => None,
    });

    // Find out if we should cache the query on disk
    let cache = query.attrs.0.iter().find_map(|attr| match attr {
        QueryModifier::Cache(tcx, expr) => Some((tcx, expr)),
        _ => None,
    }).map(|(tcx, expr)| {
        let try_load_from_disk = if let Some((tcx, id, block)) = load_cached {
            quote! {
                #[inline]
                fn try_load_from_disk(
                    #tcx: TyCtxt<'_, 'tcx, 'tcx>,
                    #id: SerializedDepNodeIndex
                ) -> Option<Self::Value> {
                    #block
                }
            }
        } else {
            quote! {
                #[inline]
                fn try_load_from_disk(
                    tcx: TyCtxt<'_, 'tcx, 'tcx>,
                    id: SerializedDepNodeIndex
                ) -> Option<Self::Value> {
                    tcx.queries.on_disk_cache.try_load_query_result(tcx, id)
                }
            }
        };

        let tcx = tcx.as_ref().map(|t| quote! { #t }).unwrap_or(quote! { _ });
        quote! {
            #[inline]
            fn cache_on_disk(#tcx: TyCtxt<'_, 'tcx, 'tcx>, #key: Self::Key) -> bool {
                #expr
            }

            #try_load_from_disk
        }
    });

    if cache.is_none() && load_cached.is_some() {
        panic!("load_cached modifier on query `{}` without a cache modifier", name);
    }

    let desc = query.attrs.0.iter().find_map(|attr| match attr {
        QueryModifier::Desc(tcx, desc) => Some((tcx, desc)),
        _ => None,
    }).map(|(tcx, desc)| {
        let tcx = tcx.as_ref().map(|t| quote! { #t }).unwrap_or(quote! { _ });
        quote! {
            fn describe(
                #tcx: TyCtxt<'_, '_, '_>,
                #key: #arg,
            ) -> Cow<'static, str> {
                format!(#desc).into()
            }
        }
    });

    if desc.is_some() || cache.is_some() {
        let cache = cache.unwrap_or(quote! {});
        let desc = desc.unwrap_or(quote! {});

        impls.extend(quote! {
            impl<'tcx> QueryDescription<'tcx> for queries::#name<'tcx> {
                #desc
                #cache
            }
        });
    }
}

pub fn rustc_queries(input: TokenStream) -> TokenStream {
    let groups = parse_macro_input!(input as List<Group>);

    let mut query_stream = quote! {};
    let mut query_description_stream = quote! {};
    let mut dep_node_def_stream = quote! {};
    let mut dep_node_force_stream = quote! {};

    for group in groups.0 {
        let mut group_stream = quote! {};
        for query in &group.queries.0 {
            let name = &query.name;
            let arg = &query.arg;
            let result_full = &query.result;
            let result = match query.result {
                ReturnType::Default => quote! { -> () },
                _ => quote! { #result_full },
            };

            // Look for a fatal_cycle modifier to pass on
            let fatal_cycle = query.attrs.0.iter().find_map(|attr| match attr {
                QueryModifier::FatalCycle => Some(()),
                _ => None,
            }).map(|_| quote! { fatal_cycle }).unwrap_or(quote! {});

            // Add the query to the group
            group_stream.extend(quote! {
                [#fatal_cycle] fn #name: #name(#arg) #result,
            });

            add_query_description_impl(query, &mut query_description_stream);

            // Create a dep node for the query
            dep_node_def_stream.extend(quote! {
                [] #name(#arg),
            });

            // Add a match arm to force the query given the dep node
            dep_node_force_stream.extend(quote! {
                DepKind::#name => {
                    if let Some(key) = RecoverKey::recover($tcx, $dep_node) {
                        force_ex!($tcx, #name, key);
                    } else {
                        return false;
                    }
                }
            });
        }
        let name = &group.name;
        query_stream.extend(quote! {
            #name { #group_stream },
        });
    }
    TokenStream::from(quote! {
        macro_rules! rustc_query_append {
            ([$($macro:tt)*][$($other:tt)*]) => {
                $($macro)* {
                    $($other)*

                    #query_stream

                }
            }
        }
        macro_rules! rustc_dep_node_append {
            ([$($macro:tt)*][$($other:tt)*]) => {
                $($macro)*(
                    $($other)*

                    #dep_node_def_stream
                );
            }
        }
        macro_rules! rustc_dep_node_force {
            ([$dep_node:expr, $tcx:expr] $($other:tt)*) => {
                match $dep_node.kind {
                    $($other)*

                    #dep_node_force_stream
                }
            }
        }
        #query_description_stream
    })
}
