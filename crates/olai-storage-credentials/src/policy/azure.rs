//! Azure SAS scoping.
//!
//! A SAS is scoped by its signed resource (`sr`): a whole container, or on accounts with
//! a hierarchical namespace (ADLS Gen2) a directory and everything below it, with the
//! directory depth in `sdd`. [`sas_resource`] picks the narrowest form for a scope, and
//! [`sas_permissions`] the `sp` permission string for an access level.
//!
//! Signing stays with the minter (`olai-http`'s `azure::sas`); this module only decides
//! what to sign.

use crate::error::{Error, Result};
use crate::location::{StorageProvider, StorageScope};
use crate::model::AccessLevel;

/// The `sp` permissions for `access`, in the canonical order Azure requires.
///
/// - [`AccessLevel::Read`]: read, list (`rl`).
/// - [`AccessLevel::ReadWrite`]: read, add, create, write, delete, list (`racwdl`).
pub fn sas_permissions(access: AccessLevel) -> &'static str {
    match access {
        AccessLevel::Read => "rl",
        AccessLevel::ReadWrite => "racwdl",
    }
}

/// What a SAS is signed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SasResource {
    /// The whole container (`sr=c`).
    Container,
    /// A directory and everything below it (`sr=d`, `sdd=<depth>`). Requires a
    /// hierarchical-namespace (ADLS Gen2) account.
    Directory {
        /// Number of path segments from the container to the directory.
        depth: usize,
    },
}

impl SasResource {
    /// The `sr` query value.
    pub fn signed_resource(self) -> &'static str {
        match self {
            Self::Container => "c",
            Self::Directory { .. } => "d",
        }
    }

    /// The `sdd` query value, for directory SAS.
    pub fn signed_directory_depth(self) -> Option<usize> {
        match self {
            Self::Container => None,
            Self::Directory { depth } => Some(depth),
        }
    }
}

/// The narrowest SAS resource covering `scope`: the container when the scope is the
/// whole container, otherwise the scope's directory.
///
/// Fails if `scope` is not an Azure location.
pub fn sas_resource(scope: &StorageScope) -> Result<SasResource> {
    let location = scope.location();
    if location.provider() != StorageProvider::Azure {
        return Err(Error::invalid_location(
            location.to_string(),
            "SAS scoping applies to Azure scopes only",
        ));
    }
    Ok(match scope.depth() {
        0 => SasResource::Container,
        depth => SasResource::Directory { depth },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource(scope: &str) -> Result<SasResource> {
        sas_resource(&scope.parse().unwrap())
    }

    #[test]
    fn permissions() {
        assert_eq!(sas_permissions(AccessLevel::Read), "rl");
        assert_eq!(sas_permissions(AccessLevel::ReadWrite), "racwdl");
    }

    #[test]
    fn container_and_directory_scopes() {
        let container = resource("abfss://ctr@acct.dfs.core.windows.net/").unwrap();
        assert_eq!(container, SasResource::Container);
        assert_eq!(container.signed_resource(), "c");
        assert_eq!(container.signed_directory_depth(), None);

        let dir = resource("https://acct.blob.core.windows.net/ctr/warehouse/tab/").unwrap();
        assert_eq!(dir, SasResource::Directory { depth: 2 });
        assert_eq!(dir.signed_resource(), "d");
        assert_eq!(dir.signed_directory_depth(), Some(2));
    }

    #[test]
    fn rejects_other_providers() {
        assert!(resource("s3://bucket/tab").is_err());
    }
}
