//! Macros for Actix system and runtime.
//!
//! The [`actix-rt`](https://docs.rs/actix-rt) crate must be available for macro output to compile.
//!
//! # Entry-point
//! See docs for the [`#[main]`](macro@main) macro.
//!
//! # Tests
//! See docs for the [`#[test]`](macro@test) macro.

#![deny(rust_2018_idioms, nonstandard_style)]
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]

use proc_macro::TokenStream;
use quote::quote;

/// Marks async entry-point function to be executed by Actix system.
///
/// # Examples
/// ```
/// #[actix_rt::main]
/// async fn main() {
///     println!("Hello world");
/// }
/// ```
#[allow(clippy::needless_doctest_main)]
#[proc_macro_attribute]
#[cfg(not(test))] // Work around for rust-lang/rust#62127
pub fn main(args: TokenStream, item: TokenStream) -> TokenStream {
    let mut input = syn::parse_macro_input!(item as syn::ItemFn);
    let args = syn::parse_macro_input!(args as syn::AttributeArgs);

    let attrs = &input.attrs;
    let vis = &input.vis;
    let sig = &mut input.sig;
    let body = &input.block;

    if sig.asyncness.is_none() {
        return syn::Error::new_spanned(
            sig.fn_token,
            "the async keyword is missing from the function declaration",
        )
        .to_compile_error()
        .into();
    }

    let mut system = syn::parse_str::<syn::Path>("::actix_rt::System").unwrap();

    for arg in &args {
        match arg {
            syn::NestedMeta::Meta(syn::Meta::NameValue(syn::MetaNameValue {
                lit: syn::Lit::Str(lit),
                path,
                ..
            })) => match path
                .get_ident()
                .map(|i| i.to_string().to_lowercase())
                .as_deref()
            {
                Some("system") => match lit.parse() {
                    Ok(path) => system = path,
                    Err(_) => {
                        return syn::Error::new_spanned(lit, "Expected path")
                            .to_compile_error()
                            .into();
                    }
                },
                _ => {
                    return syn::Error::new_spanned(arg, "Unknown attribute specified")
                        .to_compile_error()
                        .into();
                }
            },
            _ => {
                return syn::Error::new_spanned(arg, "Unknown attribute specified")
                    .to_compile_error()
                    .into();
            }
        }
    }

    sig.asyncness = None;

    (quote! {
        #(#attrs)*
        #vis #sig {
            <#system>::new().block_on(async move { #body })
        }
    })
    .into()
}

/// Marks async test function to be executed in an Actix system.
///
/// # Examples
/// ```
/// #[actix_rt::test]
/// async fn my_test() {
///     assert!(true);
/// }
/// ```
#[proc_macro_attribute]
pub fn test(_: TokenStream, item: TokenStream) -> TokenStream {
    let mut input = syn::parse_macro_input!(item as syn::ItemFn);
    let attrs = &input.attrs;
    let vis = &input.vis;
    let sig = &mut input.sig;
    let body = &input.block;
    let mut has_test_attr = false;

    for attr in attrs {
        if attr.path.is_ident("test") {
            has_test_attr = true;
        }
    }

    if sig.asyncness.is_none() {
        return syn::Error::new_spanned(
            input.sig.fn_token,
            "the async keyword is missing from the function declaration",
        )
        .to_compile_error()
        .into();
    }

    sig.asyncness = None;

    let missing_test_attr = if has_test_attr {
        quote!()
    } else {
        quote!(#[test])
    };

    (quote! {
        #missing_test_attr
        #(#attrs)*
        #vis #sig {
            actix_rt::System::new()
                .block_on(async { #body })
        }
    })
    .into()
}
