//! Azure shared access signature (SAS) helpers.
//!
//! Servers send a SAS as a URL query string (`sv=...&se=...&sig=...`), already
//! percent-encoded. [`parse_sas`] decodes it exactly once into `(key, value)` pairs, the
//! form [`StorageCredentials::AzureSas`](crate::StorageCredentials::AzureSas) holds and
//! `object_store` encodes again when signing. Keeping the encoded string and handing it
//! on is what produced double-encoded signatures in earlier implementations.

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use percent_encoding::percent_decode_str;

use crate::error::{Error, Result};

/// Splits a SAS query string into percent-decoded `(key, value)` pairs.
///
/// A leading `?` is ignored, and so are empty `&&` components. Each key and value is
/// decoded on its own, after splitting, so an encoded `%26` or `%3D` inside a value
/// stays inside it. `+` is kept literally: SAS values are percent-encoded, not
/// form-encoded, and a base64 signature may contain `+`.
///
/// Fails if the token is empty, has a component without `=`, is not valid UTF-8 once
/// decoded, or has no `sig` parameter.
pub fn parse_sas(token: &str) -> Result<Vec<(String, String)>> {
    let token = token.strip_prefix('?').unwrap_or(token);
    let decode = |part: &str| {
        percent_decode_str(part)
            .decode_utf8()
            .map(|s| s.into_owned())
            .map_err(|_| Error::invalid_credential("SAS token is not valid UTF-8"))
    };

    let mut pairs = Vec::new();
    for component in token.split('&').filter(|c| !c.is_empty()) {
        let (key, value) = component
            .split_once('=')
            .ok_or_else(|| Error::invalid_credential("SAS token has a parameter without `=`"))?;
        pairs.push((decode(key)?, decode(value)?));
    }
    if !pairs.iter().any(|(k, _)| k == "sig") {
        return Err(Error::invalid_credential(
            "SAS token has no `sig` parameter",
        ));
    }
    Ok(pairs)
}

/// Encodes decoded SAS pairs back into a query string, the inverse of [`parse_sas`].
///
/// Everything but RFC 3986 unreserved characters is percent-encoded, so values holding
/// `&`, `=`, `+` or `/` survive a round trip.
pub fn encode_sas(pairs: &[(String, String)]) -> String {
    use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
    const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'.')
        .remove(b'_')
        .remove(b'~');
    pairs
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                utf8_percent_encode(k, COMPONENT),
                utf8_percent_encode(v, COMPONENT)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// The expiry (`se`) of a decoded SAS, if present and well-formed.
///
/// Accepts the ISO 8601 forms Azure documents for `se`: `YYYY-MM-DD`,
/// `YYYY-MM-DDThh:mmZ` and `YYYY-MM-DDThh:mm:ss[.fffffff]Z`. A date alone means
/// midnight UTC.
pub fn sas_expiry(pairs: &[(String, String)]) -> Option<DateTime<Utc>> {
    let se = pairs.iter().find(|(k, _)| k == "se")?.1.as_str();
    if let Ok(t) = DateTime::parse_from_rfc3339(se) {
        return Some(t.with_timezone(&Utc));
    }
    if let Ok(t) = NaiveDateTime::parse_from_str(se, "%Y-%m-%dT%H:%MZ") {
        return Some(t.and_utc());
    }
    NaiveDate::parse_from_str(se, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|t| t.and_utc())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(token: &str) -> Vec<(String, String)> {
        parse_sas(token).unwrap()
    }

    #[test]
    fn decodes_each_component_once() {
        let p = pairs("?sv=2022-11-02&se=2030-01-01T00%3A00%3A00Z&sr=c&sp=rl&sig=ab%2Bc%2Fd%3D");
        assert_eq!(
            p,
            [
                ("sv", "2022-11-02"),
                ("se", "2030-01-01T00:00:00Z"),
                ("sr", "c"),
                ("sp", "rl"),
                ("sig", "ab+c/d="),
            ]
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
        );
    }

    #[test]
    fn encoded_separators_stay_inside_values() {
        // Decoding the whole string before splitting (as one earlier implementation did)
        // would split this signature in two.
        let p = pairs("sig=a%26b%3Dc&sv=1");
        assert_eq!(p[0], ("sig".to_owned(), "a&b=c".to_owned()));
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn encode_inverts_parse() {
        let token = "sv=2022-11-02&se=2030-01-01T00%3A00%3A00Z&sig=ab%2Bc%2Fd%3D%26";
        let p = pairs(token);
        assert_eq!(encode_sas(&p), token);
        assert_eq!(pairs(&encode_sas(&p)), p);
    }

    #[test]
    fn plus_is_literal() {
        assert_eq!(pairs("sig=a+b")[0].1, "a+b");
    }

    #[test]
    fn rejects_malformed_tokens() {
        for bad in ["", "?", "sv=1", "sv=1&sig", "sig=%FF"] {
            assert!(parse_sas(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn expiry_forms() {
        let at = |s: &str| sas_expiry(&pairs(&format!("se={s}&sig=x"))).map(|t| t.to_rfc3339());
        assert_eq!(
            at("2030-01-02T03:04:05Z").as_deref(),
            Some("2030-01-02T03:04:05+00:00")
        );
        assert_eq!(
            at("2030-01-02T03:04:05.1234567Z").as_deref(),
            Some("2030-01-02T03:04:05.123456700+00:00")
        );
        assert_eq!(
            at("2030-01-02T03:04Z").as_deref(),
            Some("2030-01-02T03:04:00+00:00")
        );
        assert_eq!(
            at("2030-01-02").as_deref(),
            Some("2030-01-02T00:00:00+00:00")
        );
        assert_eq!(at("soon"), None);
        assert_eq!(sas_expiry(&pairs("sig=x")), None);
    }
}
