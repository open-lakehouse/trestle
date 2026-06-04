//! Token-stream formatting helpers shared across the code generators.
//!
//! These turn `proc_macro2::TokenStream`s into pretty-printed Rust source and build small
//! attribute fragments (e.g. `#[doc = "..."]`). They are split out of [`super`] so the
//! parent module can focus on generation orchestration.

use proc_macro2::TokenStream;
use quote::quote;
use syn::File;

use crate::error::{Error, Result};

/// Convert optional documentation into `#[doc = "..."]` token stream attributes.
pub(crate) fn doc_tokens(documentation: Option<&str>) -> TokenStream {
    let Some(doc) = documentation else {
        return quote! {};
    };
    let doc = doc.trim();
    if doc.is_empty() {
        return quote! {};
    }
    let attrs: Vec<TokenStream> = doc
        .lines()
        .map(|line| {
            let line = line.trim();
            if line.is_empty() {
                quote! { #[doc = ""] }
            } else {
                let spaced = format!(" {}", line);
                quote! { #[doc = #spaced] }
            }
        })
        .collect();
    quote! { #(#attrs)* }
}

/// Parse `tokens` as a Rust source file and pretty-print it.
///
/// Hard-fails with [`Error::GeneratedParse`] (carrying the offending token string) when the
/// generated tokens don't parse — surfacing generator bugs instead of silently emitting an
/// empty/comment-only module that hides them behind uncompilable output.
pub(crate) fn format_tokens(tokens: TokenStream) -> Result<String> {
    let tokens_string = tokens.to_string();
    let syntax_tree = syn::parse2::<File>(tokens).map_err(|source| Error::GeneratedParse {
        tokens: tokens_string,
        source,
    })?;
    Ok(prettyplease::unparse(&syntax_tree))
}

/// Pretty-print `tokens` known to be statically valid (fixed hand-written templates).
///
/// Used by the module-stitching helpers that emit fixed `quote!` output. Panics if the
/// tokens fail to parse, which would indicate a bug in this crate's own templates rather
/// than in proto-derived generation.
pub(crate) fn format_tokens_static(tokens: TokenStream) -> String {
    format_tokens(tokens).expect("internal template tokens must always parse")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_tokens_pretty_prints_valid_input() {
        let out = format_tokens(quote! { pub fn answer() -> i32 { 42 } }).expect("valid tokens");
        assert!(out.contains("pub fn answer"), "output: {out}");
    }

    /// 1.4 — `format_tokens` must hard-fail (with the offending token string) on tokens that
    /// don't parse as a Rust file, rather than silently emitting an empty/comment-only module.
    #[test]
    fn format_tokens_hard_fails_on_unparsable_tokens() {
        // `struct` with no name is a valid `TokenStream` but not a valid Rust file.
        let err = format_tokens(quote! { struct }).expect_err("expected a parse error");
        match err {
            Error::GeneratedParse { tokens, .. } => {
                assert!(
                    tokens.contains("struct"),
                    "error should carry the offending token string: {tokens}"
                );
            }
            other => panic!("expected GeneratedParse, got {other:?}"),
        }
    }
}
