// @generated — do not edit by hand.
pub mod catalog;
pub mod query;
pub mod schema;
pub mod tag_assignments;
pub use catalog::CatalogClient;
pub use schema::SchemaClient;

#[allow(clippy::too_many_arguments, clippy::doc_lazy_continuation)]
pub mod client;
#[allow(unused_imports)]
pub use client::*;

pub mod error;
use futures::Future;

pub(super) fn stream_paginated<F, Fut, S, T>(
    state: S,
    op: F,
) -> impl futures::Stream<Item = crate::codegen::error::Result<T>>
where
    F: Fn(S, Option<String>) -> Fut + Copy,
    Fut: Future<Output = crate::codegen::error::Result<(T, S, Option<String>)>>,
{
    enum PaginationState<T> {
        Start(T),
        HasMore(T, String),
        Done,
    }
    futures::stream::unfold(
        PaginationState::Start(state),
        move |state| async move {
            let (s, page_token) = match state {
                PaginationState::Start(s) => (s, None),
                PaginationState::HasMore(s, page_token) if !page_token.is_empty() => {
                    (s, Some(page_token))
                }
                _ => {
                    return None;
                }
            };
            let (resp, s, continuation) = match op(s, page_token).await {
                Ok(resp) => resp,
                Err(e) => return Some((Err(e), PaginationState::Done)),
            };
            let next_state = match continuation {
                Some(token) => PaginationState::HasMore(s, token),
                None => PaginationState::Done,
            };
            Some((Ok(resp), next_state))
        },
    )
}

/// Percent-encode a URL path value. `keep_slash` keeps `/` (for a multi-segment value)
/// and drops a leading one.
fn encode_path_value(value: &impl ::std::fmt::Display, keep_slash: bool) -> String {
    use ::std::fmt::Write as _;
    let value = value.to_string();
    let value = match keep_slash {
        true => value.trim_start_matches('/'),
        false => value.as_str(),
    };
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            b'/' if keep_slash => out.push('/'),
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}
