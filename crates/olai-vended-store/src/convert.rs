//! Mapping [`StorageCredentials`] onto object_store's per-cloud credential types.

use olai_storage_credentials::StorageCredentials;

use crate::error::Result;

/// An object_store credential type that a vended credential can be turned into.
///
/// Implemented for `AwsCredential`, `AzureCredential` and `GcpCredential`, behind the
/// `aws`, `azure` and `gcp` features.
pub trait FromStorageCredentials: Sized + Send + Sync + 'static {
    /// The credential kind this type expects, for error messages.
    const KIND: &'static str;

    /// Convert, or fail with [`Error::CredentialKind`](crate::Error::CredentialKind) if `credentials` is for
    /// another cloud.
    fn from_storage_credentials(credentials: &StorageCredentials) -> Result<Self>;
}

/// The variant name of `credentials`, for error messages.
#[cfg(any(feature = "aws", feature = "azure", feature = "gcp"))]
pub(crate) fn kind(credentials: &StorageCredentials) -> &'static str {
    match credentials {
        StorageCredentials::Aws { .. } => "AWS",
        StorageCredentials::AzureSas(_) => "Azure SAS",
        StorageCredentials::AzureBearer(_) => "Azure bearer",
        StorageCredentials::GcpBearer(_) => "GCP bearer",
        StorageCredentials::None => "no",
        _ => "unknown",
    }
}

#[cfg(any(feature = "aws", feature = "azure", feature = "gcp"))]
fn mismatch<T: FromStorageCredentials>(credentials: &StorageCredentials) -> crate::Error {
    crate::Error::CredentialKind {
        expected: T::KIND,
        actual: kind(credentials),
    }
}

#[cfg(feature = "aws")]
impl FromStorageCredentials for object_store::aws::AwsCredential {
    const KIND: &'static str = "AWS";

    fn from_storage_credentials(credentials: &StorageCredentials) -> Result<Self> {
        match credentials {
            StorageCredentials::Aws {
                key_id,
                secret_key,
                token,
            } => Ok(Self {
                key_id: key_id.clone(),
                secret_key: secret_key.clone(),
                token: token.clone(),
            }),
            other => Err(mismatch::<Self>(other)),
        }
    }
}

#[cfg(feature = "azure")]
impl FromStorageCredentials for object_store::azure::AzureCredential {
    const KIND: &'static str = "Azure";

    fn from_storage_credentials(credentials: &StorageCredentials) -> Result<Self> {
        match credentials {
            // Already percent-decoded, which is what `SASToken` expects.
            StorageCredentials::AzureSas(pairs) => Ok(Self::SASToken(pairs.clone())),
            StorageCredentials::AzureBearer(token) => Ok(Self::BearerToken(token.clone())),
            other => Err(mismatch::<Self>(other)),
        }
    }
}

#[cfg(feature = "gcp")]
impl FromStorageCredentials for object_store::gcp::GcpCredential {
    const KIND: &'static str = "GCP";

    fn from_storage_credentials(credentials: &StorageCredentials) -> Result<Self> {
        match credentials {
            StorageCredentials::GcpBearer(bearer) => Ok(Self {
                bearer: bearer.clone(),
            }),
            other => Err(mismatch::<Self>(other)),
        }
    }
}

#[cfg(all(test, any(feature = "aws", feature = "azure", feature = "gcp")))]
mod tests {
    use super::*;

    #[cfg(feature = "aws")]
    #[test]
    fn aws_round_trip() {
        let creds = StorageCredentials::aws("AKID", "secret", Some("tok".into()));
        let aws = object_store::aws::AwsCredential::from_storage_credentials(&creds).unwrap();
        assert_eq!(aws.key_id, "AKID");
        assert_eq!(aws.secret_key, "secret");
        assert_eq!(aws.token.as_deref(), Some("tok"));
    }

    #[cfg(feature = "azure")]
    #[test]
    fn azure_sas_and_bearer() {
        use object_store::azure::AzureCredential;

        let sas = StorageCredentials::azure_sas("sv=2022-11-02&sig=a%2Bb").unwrap();
        match AzureCredential::from_storage_credentials(&sas).unwrap() {
            AzureCredential::SASToken(pairs) => {
                assert!(pairs.contains(&("sig".to_string(), "a+b".to_string())));
            }
            other => panic!("expected SAS, got {other:?}"),
        }
        let bearer = StorageCredentials::AzureBearer("t".into());
        assert!(matches!(
            AzureCredential::from_storage_credentials(&bearer).unwrap(),
            AzureCredential::BearerToken(t) if t == "t"
        ));
    }

    #[cfg(feature = "gcp")]
    #[test]
    fn gcp_bearer() {
        let creds = StorageCredentials::GcpBearer("ya29".into());
        let gcp = object_store::gcp::GcpCredential::from_storage_credentials(&creds).unwrap();
        assert_eq!(gcp.bearer, "ya29");
    }

    #[cfg(feature = "aws")]
    #[test]
    fn mismatched_kind_is_rejected_without_leaking() {
        let creds = StorageCredentials::GcpBearer("ya29-secret".into());
        let err = object_store::aws::AwsCredential::from_storage_credentials(&creds).unwrap_err();
        let msg = err.to_string();
        assert_eq!(msg, "expected AWS credentials, got GCP bearer");
        assert!(!msg.contains("ya29-secret"));
    }
}
