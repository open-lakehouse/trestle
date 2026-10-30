//! Registered credentials: the long-lived principals a server mints from.

use std::fmt;

use serde::{Deserialize, Serialize};

const REDACTED: &str = "<redacted>";

/// A registered storage credential: the long-lived identity (or secret) that vended
/// credentials are minted from.
///
/// Not to be confused with a *vended* credential, the short-lived secret a client
/// receives. The JSON shape is Unity Catalog's `Credential` without its metadata: an
/// object with exactly one of `aws_iam_role`, `azure_service_principal`,
/// `azure_managed_identity`, `azure_storage_key` or `gcp_service_account` (also read as
/// Databricks' `databricks_gcp_service_account`). Other keys are ignored, so a full
/// `Credential` document, or one persisted by mangrove's `StorageCredential`, loads.
///
/// `Debug` redacts every secret.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "Repr", into = "Repr")]
#[non_exhaustive]
pub enum RegisteredCredential {
    /// An AWS IAM role, assumed through STS.
    AwsIamRole(AwsIamRole),
    /// An Azure service principal.
    AzureServicePrincipal(AzureServicePrincipal),
    /// An Azure managed identity of the host.
    AzureManagedIdentity(AzureManagedIdentity),
    /// An Azure storage account key.
    AzureStorageKey(AzureStorageKey),
    /// A GCP service account.
    GcpServiceAccount(GcpServiceAccount),
}

/// An AWS IAM role to assume.
#[derive(Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct AwsIamRole {
    /// The role.
    pub role_arn: String,
    /// The STS region; `us-east-1` when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    /// The `ExternalId` the role's trust policy requires, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    /// Static base credentials to assume the role with; the host's ambient AWS
    /// credentials when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_key_id: Option<String>,
    /// The secret of [`access_key_id`](Self::access_key_id).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_access_key: Option<String>,
    /// A session token for temporary base credentials.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_token: Option<String>,
}

/// An Azure service principal.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AzureServicePrincipal {
    /// The tenant (directory) id.
    pub directory_id: String,
    /// The application (client) id.
    pub application_id: String,
    /// How it authenticates.
    #[serde(flatten)]
    pub secret: AzureClientSecret,
}

/// How an [`AzureServicePrincipal`] authenticates.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AzureClientSecret {
    /// A client secret.
    ClientSecret(String),
    /// A federated token file (workload identity).
    FederatedTokenFile(String),
}

/// An Azure managed identity of the host.
#[derive(Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct AzureManagedIdentity {
    /// The Databricks access connector id, kept for Unity Catalog compatibility.
    pub access_connector_id: String,
    /// The ARM resource id of a user-assigned identity; the system-assigned identity
    /// when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed_identity_id: Option<String>,
}

/// An Azure storage account key.
#[derive(Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct AzureStorageKey {
    /// The storage account.
    pub account_name: String,
    /// Its base64 key.
    pub account_key: String,
}

/// A GCP service account.
///
/// Without a [`key`](Self::key), the host's own identity (application default
/// credentials, the metadata server or workload identity federation) impersonates the
/// account, and needs `roles/iam.serviceAccountTokenCreator` on it. This is the shape of
/// Databricks' `databricks_gcp_service_account`, whose other fields are ignored.
#[derive(Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GcpServiceAccount {
    /// The account's email.
    pub email: String,
    /// A key of the account to authenticate with instead of impersonating it.
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub key: Option<GcpServiceAccountKey>,
}

/// A key of a [`GcpServiceAccount`].
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GcpServiceAccountKey {
    /// The contents of a service-account JSON key file.
    ServiceAccountKey(String),
    /// The path of a service-account JSON key file.
    ServiceAccountKeyFile(String),
}

impl RegisteredCredential {
    /// The kind, for errors and logs: `aws_iam_role`, `azure_service_principal`, ...
    pub fn kind(&self) -> &'static str {
        match self {
            Self::AwsIamRole(_) => "aws_iam_role",
            Self::AzureServicePrincipal(_) => "azure_service_principal",
            Self::AzureManagedIdentity(_) => "azure_managed_identity",
            Self::AzureStorageKey(_) => "azure_storage_key",
            Self::GcpServiceAccount(_) => "gcp_service_account",
        }
    }
}

impl fmt::Debug for RegisteredCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AwsIamRole(c) => f.debug_tuple("AwsIamRole").field(c).finish(),
            Self::AzureServicePrincipal(c) => {
                f.debug_tuple("AzureServicePrincipal").field(c).finish()
            }
            Self::AzureManagedIdentity(c) => {
                f.debug_tuple("AzureManagedIdentity").field(c).finish()
            }
            Self::AzureStorageKey(c) => f.debug_tuple("AzureStorageKey").field(c).finish(),
            Self::GcpServiceAccount(c) => f.debug_tuple("GcpServiceAccount").field(c).finish(),
        }
    }
}

impl fmt::Debug for AwsIamRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AwsIamRole")
            .field("role_arn", &self.role_arn)
            .field("region", &self.region)
            .field("external_id", &self.external_id.as_ref().map(|_| REDACTED))
            .field(
                "access_key_id",
                &self.access_key_id.as_ref().map(|_| REDACTED),
            )
            .field(
                "secret_access_key",
                &self.secret_access_key.as_ref().map(|_| REDACTED),
            )
            .field(
                "session_token",
                &self.session_token.as_ref().map(|_| REDACTED),
            )
            .finish()
    }
}

impl fmt::Debug for AzureServicePrincipal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AzureServicePrincipal")
            .field("directory_id", &self.directory_id)
            .field("application_id", &self.application_id)
            .field("secret", &self.secret)
            .finish()
    }
}

impl fmt::Debug for AzureClientSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClientSecret(_) => f.debug_tuple("ClientSecret").field(&REDACTED).finish(),
            // A path, not a secret.
            Self::FederatedTokenFile(path) => {
                f.debug_tuple("FederatedTokenFile").field(path).finish()
            }
        }
    }
}

impl fmt::Debug for AzureManagedIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AzureManagedIdentity")
            .field("access_connector_id", &self.access_connector_id)
            .field("managed_identity_id", &self.managed_identity_id)
            .finish()
    }
}

impl fmt::Debug for AzureStorageKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AzureStorageKey")
            .field("account_name", &self.account_name)
            .field("account_key", &REDACTED)
            .finish()
    }
}

impl fmt::Debug for GcpServiceAccount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GcpServiceAccount")
            .field("email", &self.email)
            .field("key", &self.key)
            .finish()
    }
}

impl fmt::Debug for GcpServiceAccountKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ServiceAccountKey(_) => {
                f.debug_tuple("ServiceAccountKey").field(&REDACTED).finish()
            }
            // A path, not a secret.
            Self::ServiceAccountKeyFile(path) => {
                f.debug_tuple("ServiceAccountKeyFile").field(path).finish()
            }
        }
    }
}

/// The wire shape: one optional field per kind.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Repr {
    #[serde(skip_serializing_if = "Option::is_none")]
    aws_iam_role: Option<AwsIamRole>,
    #[serde(skip_serializing_if = "Option::is_none")]
    azure_service_principal: Option<AzureServicePrincipal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    azure_managed_identity: Option<AzureManagedIdentity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    azure_storage_key: Option<AzureStorageKey>,
    #[serde(
        alias = "databricks_gcp_service_account",
        skip_serializing_if = "Option::is_none"
    )]
    gcp_service_account: Option<GcpServiceAccount>,
}

impl TryFrom<Repr> for RegisteredCredential {
    type Error = String;

    fn try_from(repr: Repr) -> Result<Self, Self::Error> {
        let mut kinds = [
            repr.aws_iam_role.map(Self::AwsIamRole),
            repr.azure_service_principal
                .map(Self::AzureServicePrincipal),
            repr.azure_managed_identity.map(Self::AzureManagedIdentity),
            repr.azure_storage_key.map(Self::AzureStorageKey),
            repr.gcp_service_account.map(Self::GcpServiceAccount),
        ]
        .into_iter()
        .flatten();
        match (kinds.next(), kinds.next()) {
            (Some(credential), None) => Ok(credential),
            (None, _) => Err("no supported storage credential type is set".to_owned()),
            (Some(_), Some(_)) => Err("more than one storage credential type is set".to_owned()),
        }
    }
}

impl From<RegisteredCredential> for Repr {
    fn from(credential: RegisteredCredential) -> Self {
        let mut repr = Self::default();
        match credential {
            RegisteredCredential::AwsIamRole(c) => repr.aws_iam_role = Some(c),
            RegisteredCredential::AzureServicePrincipal(c) => {
                repr.azure_service_principal = Some(c)
            }
            RegisteredCredential::AzureManagedIdentity(c) => repr.azure_managed_identity = Some(c),
            RegisteredCredential::AzureStorageKey(c) => repr.azure_storage_key = Some(c),
            RegisteredCredential::GcpServiceAccount(c) => repr.gcp_service_account = Some(c),
        }
        repr
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn loads_unity_catalog_credential_documents() {
        // A UC `Credential` with metadata fields, as mangrove persists it.
        let role: RegisteredCredential = serde_json::from_value(json!({
            "name": "prod",
            "comment": "ignored",
            "aws_iam_role": { "role_arn": "arn:aws:iam::1:role/r", "region": "eu-west-1" },
        }))
        .unwrap();
        assert!(
            matches!(&role, RegisteredCredential::AwsIamRole(r) if r.role_arn == "arn:aws:iam::1:role/r")
        );

        let principal: RegisteredCredential = serde_json::from_value(json!({
            "azure_service_principal": {
                "directory_id": "tenant", "application_id": "app", "client_secret": "s",
            },
        }))
        .unwrap();
        assert!(matches!(
            &principal,
            RegisteredCredential::AzureServicePrincipal(p)
                if p.secret == AzureClientSecret::ClientSecret("s".into())
        ));

        let federated: RegisteredCredential = serde_json::from_value(json!({
            "azure_service_principal": {
                "directory_id": "tenant", "application_id": "app",
                "federated_token_file": "/var/run/token",
            },
        }))
        .unwrap();
        assert_eq!(federated.kind(), "azure_service_principal");
    }

    #[test]
    fn loads_gcp_service_accounts() {
        let databricks: RegisteredCredential = serde_json::from_value(json!({
            "name": "gcs",
            "databricks_gcp_service_account": {
                "email": "vend@project.iam.gserviceaccount.com",
                "credential_id": "ignored",
                "private_key_id": "ignored",
            },
        }))
        .unwrap();
        let impersonated = RegisteredCredential::GcpServiceAccount(GcpServiceAccount {
            email: "vend@project.iam.gserviceaccount.com".into(),
            key: None,
        });
        assert_eq!(databricks, impersonated);
        assert_eq!(
            serde_json::to_value(&impersonated).unwrap(),
            json!({ "gcp_service_account": { "email": "vend@project.iam.gserviceaccount.com" } })
        );

        for (json, key) in [
            (
                json!({ "email": "sa@p", "service_account_key": "{}" }),
                GcpServiceAccountKey::ServiceAccountKey("{}".into()),
            ),
            (
                json!({ "email": "sa@p", "service_account_key_file": "/etc/sa.json" }),
                GcpServiceAccountKey::ServiceAccountKeyFile("/etc/sa.json".into()),
            ),
        ] {
            let document = json!({ "gcp_service_account": json });
            let credential: RegisteredCredential =
                serde_json::from_value(document.clone()).unwrap();
            assert_eq!(
                credential,
                RegisteredCredential::GcpServiceAccount(GcpServiceAccount {
                    email: "sa@p".into(),
                    key: Some(key),
                })
            );
            assert_eq!(serde_json::to_value(&credential).unwrap(), document);
        }
    }

    #[test]
    fn round_trips_and_rejects_ambiguity() {
        let key = RegisteredCredential::AzureStorageKey(AzureStorageKey {
            account_name: "acct".into(),
            account_key: "a2V5".into(),
        });
        let json = serde_json::to_value(&key).unwrap();
        assert_eq!(
            json,
            json!({ "azure_storage_key": { "account_name": "acct", "account_key": "a2V5" } })
        );
        assert_eq!(
            serde_json::from_value::<RegisteredCredential>(json).unwrap(),
            key
        );

        for invalid in [
            json!({}),
            json!({ "aws_iam_role": { "role_arn": "a" }, "azure_storage_key": {} }),
        ] {
            assert!(serde_json::from_value::<RegisteredCredential>(invalid).is_err());
        }
    }

    #[test]
    fn debug_redacts_secrets() {
        let credentials = [
            RegisteredCredential::AwsIamRole(AwsIamRole {
                role_arn: "arn".into(),
                external_id: Some("SECRET-EXT".into()),
                access_key_id: Some("SECRET-AKID".into()),
                secret_access_key: Some("SECRET-KEY".into()),
                session_token: Some("SECRET-TOKEN".into()),
                ..AwsIamRole::default()
            }),
            RegisteredCredential::AzureServicePrincipal(AzureServicePrincipal {
                directory_id: "tenant".into(),
                application_id: "app".into(),
                secret: AzureClientSecret::ClientSecret("SECRET-CLIENT".into()),
            }),
            RegisteredCredential::AzureStorageKey(AzureStorageKey {
                account_name: "acct".into(),
                account_key: "SECRET-ACCOUNT".into(),
            }),
            RegisteredCredential::GcpServiceAccount(GcpServiceAccount {
                email: "sa@p".into(),
                key: Some(GcpServiceAccountKey::ServiceAccountKey("SECRET-KEY".into())),
            }),
        ];
        for credential in credentials {
            let debug = format!("{credential:?}");
            assert!(!debug.contains("SECRET"), "{debug}");
        }
    }
}
