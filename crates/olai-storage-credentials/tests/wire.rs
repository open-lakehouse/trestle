//! Wire adapters against payload fixtures.
//!
//! Each fixture is a response as servers send it (see `tests/fixtures/<protocol>/`).
//! The tests read it through the client direction, check the model, then serialize the
//! model back through the server direction and read it again, which must give the
//! same model.
//!
//! Run with `cargo test -p olai-storage-credentials --features wire`.
#![cfg(any(
    feature = "unity",
    feature = "delta-api",
    feature = "iceberg",
    feature = "sharing"
))]

use chrono::{DateTime, Utc};
use olai_storage_credentials::wire::VendContext;
use olai_storage_credentials::{AccessLevel, StorageCredentials, StorageScope, VendedCredential};
use serde::de::DeserializeOwned;

/// 2030-01-01T00:00:00Z, the expiry most fixtures use.
const EXPIRY_MS: i64 = 1_893_456_000_000;

fn fixture<T: DeserializeOwned>(path: &str) -> T {
    let full = format!("{}/tests/fixtures/{path}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("{full}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{full}: {e}"))
}

fn ctx(scope: &str, access: AccessLevel) -> VendContext {
    VendContext::new(scope.parse().unwrap(), access)
}

fn scope(s: &str) -> StorageScope {
    s.parse().unwrap()
}

fn expiry() -> Option<DateTime<Utc>> {
    DateTime::from_timestamp_millis(EXPIRY_MS)
}

fn aws(key_id: &str, token: Option<&str>) -> StorageCredentials {
    StorageCredentials::aws(
        key_id,
        format!("sk{}", &key_id[2..]),
        token.map(str::to_owned),
    )
}

/// Serialize through `to_wire`, parse the JSON again, and read it back.
fn round_trip<W>(
    vended: &VendedCredential,
    ctx: &VendContext,
    to_wire: impl Fn(&VendedCredential) -> W,
    read: impl Fn(&W, &VendContext) -> VendedCredential,
) where
    W: serde::Serialize + DeserializeOwned,
{
    let json = serde_json::to_string(&to_wire(vended)).unwrap();
    let wire: W = serde_json::from_str(&json).unwrap();
    assert_eq!(&read(&wire, ctx), vended, "round trip through {json}");
}

#[cfg(feature = "unity")]
mod unity_rest {
    use olai_storage_credentials::wire::unity;

    use super::*;

    fn read(path: &str, ctx: &VendContext) -> VendedCredential {
        let wire: unity::TemporaryCredential = fixture(path);
        let vended = wire.to_vended(ctx).unwrap();
        round_trip(
            &vended,
            ctx,
            |v| unity::TemporaryCredential::from(v),
            |w, c| w.to_vended(c).unwrap(),
        );
        vended
    }

    #[test]
    fn aws_table_credential() {
        let ctx = ctx(
            "s3://bucket/warehouse/catalog/schema/tab",
            AccessLevel::ReadWrite,
        );
        let v = read("unity/aws_table.json", &ctx);
        assert_eq!(v.credentials, aws("ak-test", Some("st-test")));
        assert_eq!(v.scope, ctx.scope);
        assert_eq!(
            v.access,
            AccessLevel::ReadWrite,
            "access comes from the request"
        );
        assert_eq!(v.expires_at, expiry());
        assert!(v.hints.is_empty(), "an empty access point is no hint");
    }

    #[test]
    fn null_expiry_access_point_and_unknown_fields() {
        let v = read(
            "unity/aws_null_expiry.json",
            &ctx("s3://bucket/warehouse/tab", AccessLevel::Read),
        );
        assert_eq!(v.expires_at, None);
        assert_eq!(
            v.hints.access_point.as_deref(),
            Some("arn:aws:s3:us-west-2:123456789012:accesspoint/lake")
        );
    }

    #[test]
    fn missing_url_takes_the_requested_scope() {
        let ctx = ctx("abfss://c@a.dfs.core.windows.net/p", AccessLevel::ReadWrite);
        let v = read("unity/azure_aad_path.json", &ctx);
        assert_eq!(
            v.credentials,
            StorageCredentials::AzureBearer("aad-test".into())
        );
        assert_eq!(v.scope, ctx.scope);
        assert_eq!(v.expires_at, None);
    }

    #[test]
    fn sas_is_decoded_once_and_string_expiry_is_read() {
        let v = read(
            "unity/azure_sas_string_expiry.json",
            &ctx(
                "https://acct.blob.core.windows.net/ctr/warehouse/tab",
                AccessLevel::Read,
            ),
        );
        let StorageCredentials::AzureSas(pairs) = &v.credentials else {
            panic!("expected a SAS, got {:?}", v.credentials);
        };
        let sig = pairs.iter().find(|(k, _)| k == "sig").unwrap();
        assert_eq!(sig.1, "ab+c/d=");
        assert_eq!(v.expires_at, expiry(), "explicit expiry wins over se=");
    }

    #[test]
    fn sas_expiry_falls_back_to_se() {
        let v = read(
            "unity/azure_sas_no_expiry.json",
            &ctx(
                "abfss://ctr@acct.dfs.core.windows.net/warehouse/tab",
                AccessLevel::Read,
            ),
        );
        assert_eq!(
            v.expires_at.unwrap().to_rfc3339(),
            "2030-06-01T12:00:00+00:00"
        );
    }

    #[test]
    fn gcp_r2_and_local() {
        let v = read(
            "unity/gcp.json",
            &ctx("gs://bucket/warehouse/tab", AccessLevel::Read),
        );
        assert_eq!(
            v.credentials,
            StorageCredentials::GcpBearer("ya29.test".into())
        );

        let v = read(
            "unity/r2.json",
            &ctx(
                "r2://bucket@acct.r2.cloudflarestorage.com/warehouse/tab",
                AccessLevel::Read,
            ),
        );
        assert_eq!(v.credentials, aws("ak-test", Some("st-test")));
        assert_eq!(
            v.hints.endpoint.as_deref(),
            Some("https://acct.r2.cloudflarestorage.com")
        );
        assert_eq!(v.hints.region.as_deref(), Some("auto"));

        let v = read(
            "unity/local.json",
            &ctx("file:///tmp/warehouse/tab", AccessLevel::ReadWrite),
        );
        assert_eq!(v.credentials, StorageCredentials::None);
        assert_eq!(v.expires_at, None, "expiration_time 0 means no expiry");
    }

    #[test]
    fn url_narrows_but_never_widens() {
        let wire: unity::TemporaryCredential = fixture("unity/aws_table.json");

        // A broader request is narrowed to the url.
        let v = wire
            .to_vended(&ctx("s3://bucket/warehouse", AccessLevel::Read))
            .unwrap();
        assert_eq!(v.scope, scope("s3://bucket/warehouse/catalog/schema/tab"));

        // A narrower request keeps its own scope.
        let narrow = ctx(
            "s3://bucket/warehouse/catalog/schema/tab/data",
            AccessLevel::Read,
        );
        assert_eq!(wire.to_vended(&narrow).unwrap().scope, narrow.scope);

        // An unrelated or sibling request is rejected.
        assert!(
            wire.to_vended(&ctx(
                "s3://bucket/warehouse/catalog/schema/tab2",
                AccessLevel::Read
            ))
            .is_err()
        );
        assert!(
            wire.to_vended(&ctx("s3://other/warehouse", AccessLevel::Read))
                .is_err()
        );
    }

    #[test]
    fn rejects_two_credentials_and_none_for_cloud_storage() {
        let wire: unity::TemporaryCredential = fixture("unity/two_credentials.json");
        assert!(
            wire.to_vended(&ctx("s3://bucket/warehouse/tab", AccessLevel::Read))
                .is_err()
        );

        let empty: unity::TemporaryCredential =
            serde_json::from_str(r#"{"url":"s3://bucket/x"}"#).unwrap();
        assert!(
            empty
                .to_vended(&ctx("s3://bucket/x", AccessLevel::Read))
                .is_err()
        );
    }

    #[test]
    fn debug_redacts() {
        let wire: unity::TemporaryCredential = fixture("unity/aws_table.json");
        let rendered = format!("{wire:?}");
        for secret in ["ak-test", "sk-test", "st-test"] {
            assert!(!rendered.contains(secret), "{rendered}");
        }
    }
}

#[cfg(feature = "sharing")]
mod open_sharing {
    use olai_storage_credentials::wire::sharing;

    use super::*;

    fn read(path: &str, ctx: &VendContext) -> VendedCredential {
        let wire: sharing::SharingTemporaryCredentials = fixture(path);
        let vended = wire.to_vended(ctx).unwrap();
        round_trip(
            &vended,
            ctx,
            |v| sharing::SharingTemporaryCredentials::try_from(v).unwrap(),
            |w, c| w.to_vended(c).unwrap(),
        );
        vended
    }

    #[test]
    fn volume_credential() {
        let v = read(
            "sharing/volume_aws.json",
            &ctx("s3://bucket/shares/vol", AccessLevel::Read),
        );
        assert_eq!(v.credentials, aws("ak-test", Some("st-test")));
        assert_eq!(v.expires_at, expiry());
    }

    #[test]
    fn camel_case_input() {
        let v = read(
            "sharing/volume_camel_case.json",
            &ctx("gs://bucket/shares/vol", AccessLevel::Read),
        );
        assert_eq!(
            v.credentials,
            StorageCredentials::GcpBearer("ya29.test".into())
        );
        assert_eq!(v.expires_at, expiry());
    }

    #[test]
    fn r2_and_proto_json_output() {
        let v = read(
            "sharing/r2.json",
            &ctx(
                "r2://bucket@acct.r2.cloudflarestorage.com/shares/vol",
                AccessLevel::Read,
            ),
        );
        let json =
            serde_json::to_value(sharing::SharingTemporaryCredentials::try_from(&v).unwrap())
                .unwrap();
        assert_eq!(
            json["expiration_time"],
            EXPIRY_MS.to_string(),
            "int64 as a string"
        );
        assert!(json.get("r2_credentials").is_some());
    }

    #[test]
    fn cannot_carry_azure_ad() {
        let v = VendedCredential::new(
            StorageCredentials::AzureBearer("t".into()),
            scope("abfss://ctr@acct.dfs.core.windows.net/vol"),
            AccessLevel::Read,
        );
        assert!(sharing::SharingTemporaryCredentials::try_from(&v).is_err());
    }
}
