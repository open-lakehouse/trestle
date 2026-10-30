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

#[cfg(feature = "delta-api")]
mod delta {
    use olai_storage_credentials::wire::delta_api;

    use super::*;

    fn read(path: &str, ctx: &VendContext) -> Vec<VendedCredential> {
        let wire: delta_api::CredentialsResponse = fixture(path);
        let vended = wire.to_vended(ctx).unwrap();
        for v in &vended {
            round_trip(
                v,
                ctx,
                |v| delta_api::StorageCredential::try_from(v).unwrap(),
                |w, c| w.to_vended(c).unwrap().unwrap(),
            );
        }
        vended
    }

    #[test]
    fn staging_table_response() {
        let ctx = ctx(
            "s3://bucket/warehouse/catalog/schema/table",
            AccessLevel::ReadWrite,
        );
        let v = read("delta_api/staging_table.json", &ctx);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].credentials, aws("ak-test", Some("st-test")));
        assert_eq!(v[0].scope, ctx.scope);
        assert_eq!(v[0].access, AccessLevel::ReadWrite);
        assert_eq!(v[0].expires_at, expiry());
    }

    #[test]
    fn access_comes_from_the_wire() {
        let v = read(
            "delta_api/credentials.json",
            &ctx("s3://bucket/path", AccessLevel::ReadWrite),
        );
        assert_eq!(v[0].access, AccessLevel::Read);
        assert_eq!(v[0].credentials, aws("ak-test", None));
    }

    #[test]
    fn null_expiry_falls_back_to_se_and_unrelated_prefixes_are_dropped() {
        let v = read(
            "delta_api/azure_and_gcs.json",
            &ctx(
                "abfss://ctr@acct.dfs.core.windows.net/warehouse/tab",
                AccessLevel::Read,
            ),
        );
        assert_eq!(v.len(), 1, "the gs:// credential is unrelated");
        assert_eq!(
            v[0].expires_at.unwrap().to_rfc3339(),
            "2030-01-01T00:00:00+00:00"
        );
    }

    #[test]
    fn rejects_a_response_with_nothing_for_the_request() {
        let wire: delta_api::CredentialsResponse = fixture("delta_api/credentials.json");
        assert!(
            wire.to_vended(&ctx("s3://bucket/other", AccessLevel::Read))
                .is_err()
        );
    }

    #[test]
    fn server_prefixes_end_in_a_slash() {
        let v = VendedCredential::new(
            aws("ak-test", None),
            scope("s3://bucket/tab"),
            AccessLevel::Read,
        );
        let wire = delta_api::StorageCredential::try_from(&v).unwrap();
        assert_eq!(wire.prefix, "s3://bucket/tab/");
        assert_eq!(wire.operation, delta_api::Operation::Read);
        assert!(!format!("{wire:?}").contains("sk-test"));
    }
}

#[cfg(feature = "iceberg")]
mod iceberg_rest {
    use olai_storage_credentials::wire::iceberg;

    use super::*;

    fn round_trip_all(vended: &[VendedCredential], ctx: &VendContext) {
        for v in vended {
            round_trip(
                v,
                ctx,
                |v| iceberg::StorageCredential::try_from(v).unwrap(),
                |w, c| w.to_vended(c).unwrap().unwrap(),
            );
        }
    }

    #[test]
    fn load_table_with_s3_storage_credentials() {
        let ctx = ctx("s3://bucket/warehouse/ns/tab", AccessLevel::ReadWrite);
        let wire: iceberg::LoadTableCredentials = fixture("iceberg/load_table_s3.json");
        let v = wire.to_vended(&ctx).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].credentials, aws("ak-test", Some("st-test")));
        assert_eq!(v[0].expires_at, expiry());
        assert_eq!(v[0].access, AccessLevel::ReadWrite);
        assert_eq!(v[0].hints.region.as_deref(), Some("us-west-2"));
        assert_eq!(
            v[0].hints.endpoint.as_deref(),
            Some("https://s3.example.internal")
        );
        assert_eq!(v[0].hints.path_style, Some(true));
        round_trip_all(&v, &ctx);
    }

    #[test]
    fn storage_credentials_layer_over_the_table_config() {
        // Endpoint, region and path style only in the table config: without layering, a
        // MinIO/R2 client would sign for, and send its keys to, s3.amazonaws.com.
        let wire: iceberg::LoadTableCredentials = serde_json::from_value(serde_json::json!({
            "config": {
                "s3.endpoint": "http://minio:9000",
                "s3.path-style-access": "true",
                "client.region": "eu-west-1",
            },
            "storage-credentials": [{
                "prefix": "s3://bucket/warehouse/ns/tab/",
                "config": {
                    "s3.access-key-id": "ak-test",
                    "s3.secret-access-key": "sk-test",
                    "s3.session-token": "st-test",
                    "client.region": "us-west-2",
                },
            }],
        }))
        .unwrap();
        let v = wire
            .to_vended(&ctx("s3://bucket/warehouse/ns/tab", AccessLevel::Read))
            .unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].credentials, aws("ak-test", Some("st-test")));
        assert_eq!(v[0].hints.endpoint.as_deref(), Some("http://minio:9000"));
        assert_eq!(v[0].hints.path_style, Some(true));
        assert_eq!(
            v[0].hints.region.as_deref(),
            Some("us-west-2"),
            "the entry's keys win"
        );
    }

    #[test]
    fn picks_the_sas_for_the_prefix_account() {
        let ctx = ctx(
            "abfss://ctr@acct.dfs.core.windows.net/warehouse/ns/tab",
            AccessLevel::Read,
        );
        let wire: iceberg::LoadTableCredentials = fixture("iceberg/load_table_adls.json");
        let v = wire.to_vended(&ctx).unwrap();
        assert_eq!(v.len(), 1);
        let StorageCredentials::AzureSas(pairs) = &v[0].credentials else {
            panic!("expected a SAS");
        };
        assert!(
            pairs.contains(&("sig".to_owned(), "YWNjdA==".to_owned())),
            "{pairs:?}"
        );
        assert_eq!(v[0].expires_at, expiry(), "the account's expiry companion");
        round_trip_all(&v, &ctx);
    }

    #[test]
    fn legacy_table_config() {
        let ctx = ctx("gs://bucket/warehouse/ns/tab", AccessLevel::Read);
        let wire: iceberg::LoadTableCredentials = fixture("iceberg/load_table_legacy_config.json");
        let v = wire.to_vended(&ctx).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(
            v[0].credentials,
            StorageCredentials::GcpBearer("ya29.test".into())
        );
        assert_eq!(v[0].scope, ctx.scope);
        assert_eq!(v[0].expires_at, expiry());
        round_trip_all(&v, &ctx);
    }

    #[test]
    fn no_credentials_without_vending() {
        let wire: iceberg::LoadTableCredentials = fixture("iceberg/load_table_remote_signing.json");
        let v = wire
            .to_vended(&ctx("s3://bucket/warehouse/ns/tab", AccessLevel::Read))
            .unwrap();
        assert!(v.is_empty());
        assert!(wire.remote_signing());
    }

    #[test]
    fn load_credentials_clamps_and_drops() {
        let ctx = ctx("s3://bucket/warehouse/ns/tab", AccessLevel::Read);
        let wire: iceberg::LoadCredentialsResponse = fixture("iceberg/load_credentials.json");
        let v = wire.to_vended(&ctx).unwrap();
        // The bucket-wide credential is clamped to the table; the data-prefix one kept
        // as is; the sibling table and the other bucket are dropped.
        let scopes: Vec<_> = v.iter().map(|v| v.scope.to_string()).collect();
        assert_eq!(
            scopes,
            [
                "s3://bucket/warehouse/ns/tab",
                "s3://bucket/warehouse/ns/tab/data"
            ]
        );
        assert_eq!(v[0].credentials, aws("ak-wide", Some("st-wide")));
        assert_eq!(v[1].credentials, aws("ak-data", Some("st-data")));
        round_trip_all(&v, &ctx);
    }

    #[test]
    fn server_direction_keys() {
        let v = VendedCredential::new(
            StorageCredentials::azure_sas("sv=1&sig=a%2Bb").unwrap(),
            scope("abfss://ctr@acct.dfs.core.windows.net/tab"),
            AccessLevel::Read,
        )
        .with_expiry(expiry());
        let wire = iceberg::StorageCredential::try_from(&v).unwrap();
        assert_eq!(wire.prefix, "abfss://ctr@acct.dfs.core.windows.net/tab/");
        assert_eq!(
            wire.config["adls.sas-token.acct.dfs.core.windows.net"],
            "sv=1&sig=a%2Bb"
        );
        assert_eq!(
            wire.config["adls.sas-token-expires-at-ms.acct.dfs.core.windows.net"],
            EXPIRY_MS.to_string()
        );
        assert!(!format!("{wire:?}").contains("a%2Bb"));

        let bearer = VendedCredential::new(
            StorageCredentials::AzureBearer("t".into()),
            scope("abfss://ctr@acct.dfs.core.windows.net/tab"),
            AccessLevel::Read,
        );
        assert!(iceberg::StorageCredential::try_from(&bearer).is_err());
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
