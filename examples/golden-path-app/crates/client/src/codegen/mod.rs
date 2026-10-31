// @generated — do not edit by hand.
#[allow(clippy::too_many_arguments, clippy::doc_lazy_continuation)]
pub mod client;
pub mod greeting;
#[allow(unused_imports)]
pub use client::*;

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
