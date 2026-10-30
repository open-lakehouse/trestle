//! Names of shared volumes: `share.schema.volume[/path]` and
//! `sharing:///share/schema/volume[/path]`.

use url::Url;

use crate::error::{Error, Result};

/// A shared volume, and an optional path below it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharingReference {
    /// The share.
    pub share: String,
    /// The schema in the share.
    pub schema: String,
    /// The volume in the schema.
    pub volume: String,
    /// A `/`-separated path below the volume's root; empty for the whole volume.
    pub path: String,
}

impl SharingReference {
    /// Parses either form:
    ///
    /// - `share.schema.volume[/path]`, the delta-sharing client convention (the part
    ///   after the profile's `#`). Exactly three dot-separated names, so names that
    ///   contain a dot need the URL form.
    /// - `sharing:///share/schema/volume[/path]`, percent-decoded segment by segment.
    pub fn parse(reference: &str) -> Result<Self> {
        if reference.starts_with("sharing:") {
            return Self::parse_url(reference);
        }
        let (names, path) = reference.split_once('/').unwrap_or((reference, ""));
        match names.split('.').collect::<Vec<_>>().as_slice() {
            [share, schema, volume] if [share, schema, volume].iter().all(|n| !n.is_empty()) => {
                Ok(Self {
                    share: (*share).to_owned(),
                    schema: (*schema).to_owned(),
                    volume: (*volume).to_owned(),
                    path: normalize(path.split('/').map(str::to_owned)),
                })
            }
            _ => Err(invalid(
                reference,
                "expected `share.schema.volume[/path]` or `sharing:///share/schema/volume[/path]`",
            )),
        }
    }

    fn parse_url(reference: &str) -> Result<Self> {
        let url = Url::parse(reference).map_err(|e| invalid(reference, &e.to_string()))?;
        if url.host_str().is_some_and(|host| !host.is_empty()) {
            return Err(invalid(
                reference,
                "the server comes from the profile; write `sharing:///share/...` with no host",
            ));
        }
        let segments = url
            .path_segments()
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .map(|s| {
                percent_encoding::percent_decode_str(s)
                    .decode_utf8()
                    .map(|s| s.into_owned())
                    .map_err(|_| invalid(reference, "invalid percent-encoding"))
            })
            .collect::<Result<Vec<_>>>()?;
        match segments.as_slice() {
            [share, schema, volume, rest @ ..] => Ok(Self {
                share: share.clone(),
                schema: schema.clone(),
                volume: volume.clone(),
                path: normalize(rest.iter().cloned()),
            }),
            _ => Err(invalid(
                reference,
                "`sharing:///share/schema/volume[/path]` requires all three names",
            )),
        }
    }
}

/// `segments` joined by `/`, without empty segments.
fn normalize(segments: impl Iterator<Item = String>) -> String {
    segments
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

fn invalid(reference: &str, reason: &str) -> Error {
    Error::InvalidReference(format!("`{reference}`: {reason}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume(share: &str, schema: &str, volume: &str, path: &str) -> SharingReference {
        SharingReference {
            share: share.into(),
            schema: schema.into(),
            volume: volume.into(),
            path: path.into(),
        }
    }

    #[test]
    fn dotted_names() {
        assert_eq!(
            SharingReference::parse("sales.raw.landing").unwrap(),
            volume("sales", "raw", "landing", "")
        );
        assert_eq!(
            SharingReference::parse("sales.raw.landing/2024//day=1/").unwrap(),
            volume("sales", "raw", "landing", "2024/day=1")
        );
        for bad in ["sales.raw", "a.b.c.d", "sales..landing", ""] {
            assert!(SharingReference::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn urls() {
        assert_eq!(
            SharingReference::parse("sharing:///sales/raw.v2/landing/my%20dir/f").unwrap(),
            volume("sales", "raw.v2", "landing", "my dir/f")
        );
        assert!(SharingReference::parse("sharing:///sales/raw").is_err());
        assert!(SharingReference::parse("sharing://host/sales/raw/landing").is_err());
    }
}
