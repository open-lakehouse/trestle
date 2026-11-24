//! The keyrings [`Keyring`] builds are the ones the secret server reads: through import,
//! rotation and retirement, the server parses each, wraps under its active version, opens what
//! older versions (and an imported key's alias) sealed, and refuses retired versions.

use olai_secret_server::Keyring as ServerKeyring;
use olai_stack_runtime::Keyring;

/// `openssl rand -base64 32`-style material for an imported key.
const IMPORTED: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=";
const KEY: &str = "server-kek";

fn server(ring: &Keyring) -> ServerKeyring {
    ServerKeyring::from_json(KEY, ring.to_secret().expose().as_bytes()).unwrap()
}

#[tokio::test]
async fn the_secret_server_reads_every_keyring_the_runtime_writes() {
    let dek = [7u8; 32];

    // Imported with the id it had as a `LocalKeyProvider` KEK.
    let mut ring = Keyring::import(IMPORTED, Some("kek-1")).unwrap();
    let v1 = server(&ring);
    assert_eq!((v1.active(), v1.min_decrypt()), (1, 1));
    let (version, under_v1) = v1.wrap(&dek, 0, "").await.unwrap();
    assert_eq!(version, 1);
    let (_, under_alias) = v1.wrap(&dek, 0, "kek-1").await.unwrap();

    // Rotated: wraps go to v2, v1 and the alias still open.
    ring.rotate().unwrap();
    let v2 = server(&ring);
    assert_eq!((v2.active(), v2.min_decrypt()), (2, 1));
    assert_eq!(v2.wrap(&dek, 0, "").await.unwrap().0, 2);
    assert_eq!(*v2.unwrap(&under_v1, 1, "").await.unwrap(), dek);
    assert_eq!(*v2.unwrap(&under_alias, 0, "kek-1").await.unwrap(), dek);

    // Retired below 2: the material stays, v1 no longer opens.
    ring.retire_below(2).unwrap();
    let retired = server(&ring);
    assert_eq!(retired.versions().collect::<Vec<_>>(), [1, 2]);
    assert_eq!(retired.min_decrypt(), 2);
    assert!(retired.unwrap(&under_v1, 1, "").await.is_err());
    assert!(retired.unwrap(&under_alias, 0, "kek-1").await.is_err());

    // A generated keyring is one the server reads too.
    assert_eq!(server(&Keyring::generate().unwrap()).active(), 1);
}
