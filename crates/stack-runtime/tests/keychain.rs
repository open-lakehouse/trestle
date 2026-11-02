//! The OS credential store: `cargo test -p olai-stack-runtime --features keychain --test
//! keychain -- --ignored`. Writes, reads and deletes one entry under a throwaway service.
#![cfg(feature = "keychain")]

use olai_stack_runtime::{KeychainProvider, SecretError, SecretProvider, SecretValue};

#[test]
#[ignore = "touches the OS credential store"]
fn a_secret_round_trips_through_the_keychain() {
    let service = format!("trestle-test-{}", std::process::id());
    let keychain = KeychainProvider::new(&service);
    keychain
        .set("probe", &SecretValue::new("keychain-value"))
        .unwrap();
    assert_eq!(keychain.get("probe").unwrap().expose(), "keychain-value");
    keychain.delete("probe").unwrap();
    assert!(matches!(
        keychain.get("probe"),
        Err(SecretError::Missing { .. })
    ));
    // Deleting what isn't there is fine.
    keychain.delete("probe").unwrap();
}
