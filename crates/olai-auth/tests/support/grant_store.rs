//! The `GrantStore` contract, run against every implementation.
//!
//! The store must have the groups `GROUP_STAFF` (`staff`) and `GROUP_ENG` (`eng`), with
//! `eng` in `staff`, user `alice` in `eng` and user `bob` in `staff`, and no grants.

use olai_auth::{GrantChange, GrantError, GrantStore, ObjectRef, Subject};

pub const GROUP_STAFF: &str = "0b8e6a3c-1f2d-4e5a-9b7c-8d6e5f4a3b20";
pub const GROUP_ENG: &str = "0b8e6a3c-1f2d-4e5a-9b7c-8d6e5f4a3b21";

fn folder(id: &str) -> ObjectRef {
    ObjectRef::new("Ns::Folder", id)
}

fn doc(id: &str) -> ObjectRef {
    ObjectRef::new("Ns::Doc", id)
}

fn change(subject: Subject, add: &[&str], remove: &[&str]) -> GrantChange {
    GrantChange {
        subject,
        add: add.iter().map(|p| p.to_string()).collect(),
        remove: remove.iter().map(|p| p.to_string()).collect(),
    }
}

fn privileges(grants: &[olai_auth::Grant]) -> Vec<(String, String)> {
    grants
        .iter()
        .map(|g| (g.subject.id.clone(), g.privilege.clone()))
        .collect()
}

pub async fn conformance(store: &impl GrantStore) {
    // apply / list
    let granted = store
        .apply(
            &folder("root"),
            &[
                change(Subject::user("alice"), &["READ", "WRITE"], &[]),
                change(Subject::group(GROUP_STAFF), &["USE"], &[]),
            ],
        )
        .await
        .unwrap();
    assert_eq!(
        privileges(&granted),
        [
            ("alice".into(), "READ".into()),
            ("alice".into(), "WRITE".into()),
            (GROUP_STAFF.into(), "USE".into()),
        ]
    );
    assert_eq!(store.list(&folder("root")).await.unwrap(), granted);
    // Granting again and revoking what isn't held are no-ops; a revoke applies.
    let after = store
        .apply(
            &folder("root"),
            &[change(
                Subject::user("alice"),
                &["READ"],
                &["WRITE", "OWNER"],
            )],
        )
        .await
        .unwrap();
    assert_eq!(
        privileges(&after),
        [
            ("alice".into(), "READ".into()),
            (GROUP_STAFF.into(), "USE".into()),
        ]
    );
    assert!(store.list(&folder("elsewhere")).await.unwrap().is_empty());

    // Invalid changes apply nothing.
    let bad = store
        .apply(
            &folder("root"),
            &[
                change(Subject::user("bob"), &["READ"], &[]),
                change(Subject::user("bob"), &["NOT-A-NAME"], &[]),
            ],
        )
        .await;
    assert!(
        matches!(bad, Err(GrantError::InvalidPrivilege(_))),
        "{bad:?}"
    );
    assert!(matches!(
        store
            .apply(
                &ObjectRef::new("Folder", "x"),
                &[change(Subject::user("bob"), &["READ"], &[])]
            )
            .await,
        Err(GrantError::InvalidObject(_))
    ));
    assert_eq!(store.list(&folder("root")).await.unwrap().len(), 2);

    // groups: transitive, by name, with the direct ones marked.
    let groups = store.groups(&Subject::user("alice")).await.unwrap();
    let summary: Vec<(&str, bool, Vec<String>)> = groups
        .iter()
        .map(|g| (g.name.as_str(), g.direct, g.parents.clone()))
        .collect();
    assert_eq!(
        summary,
        [
            ("eng", true, vec!["staff".to_string()]),
            ("staff", false, vec![])
        ]
    );
    assert_eq!(groups[0].id, GROUP_ENG);
    assert!(
        store
            .groups(&Subject::user("nobody"))
            .await
            .unwrap()
            .is_empty()
    );

    // effective: own grants and those of every group, on the objects asked about only.
    store
        .apply(
            &doc("root.a.d"),
            &[change(Subject::group(GROUP_ENG), &["OWNER"], &[])],
        )
        .await
        .unwrap();
    store
        .apply(
            &doc("root.b.d"),
            &[change(Subject::user("alice"), &["READ"], &[])],
        )
        .await
        .unwrap();
    let effective = store
        .effective(&Subject::user("alice"), &[folder("root"), doc("root.a.d")])
        .await
        .unwrap();
    let mut held: Vec<(String, String, String)> = effective
        .iter()
        .map(|g| {
            (
                g.object.id.clone(),
                g.subject.id.clone(),
                g.privilege.clone(),
            )
        })
        .collect();
    held.sort();
    assert_eq!(
        held,
        [
            ("root".into(), GROUP_STAFF.into(), "USE".into()),
            ("root".into(), "alice".into(), "READ".into()),
            ("root.a.d".into(), GROUP_ENG.into(), "OWNER".into()),
        ]
    );
    let bob = store
        .effective(&Subject::user("bob"), &[folder("root"), doc("root.a.d")])
        .await
        .unwrap();
    assert_eq!(privileges(&bob), [(GROUP_STAFF.into(), "USE".into())]);
    let facts = store
        .facts(&Subject::user("alice"), &[folder("root")])
        .await
        .unwrap();
    assert_eq!(facts.groups, groups);
    assert_eq!(facts.grants.len(), 2);

    // rename: the object and its descendants of the given types move.
    store
        .apply(
            &folder("rootish"),
            &[change(Subject::user("bob"), &["READ"], &[])],
        )
        .await
        .unwrap();
    assert!(matches!(
        store.rename(&folder("root"), "root.x", &["Ns::Doc"]).await,
        Err(GrantError::InvalidRename { .. })
    ));
    let moved = store
        .rename(&folder("root"), "top", &["Ns::Doc"])
        .await
        .unwrap();
    assert_eq!(moved, 4);
    assert!(store.list(&folder("root")).await.unwrap().is_empty());
    assert_eq!(store.list(&folder("top")).await.unwrap().len(), 2);
    assert_eq!(store.list(&doc("top.a.d")).await.unwrap().len(), 1);
    assert_eq!(store.list(&doc("top.b.d")).await.unwrap().len(), 1);
    // Not a descendant: only shares a prefix.
    assert_eq!(store.list(&folder("rootish")).await.unwrap().len(), 1);
    // A grant the destination already holds is kept once.
    store
        .apply(
            &folder("other"),
            &[change(Subject::user("alice"), &["READ"], &[])],
        )
        .await
        .unwrap();
    store
        .apply(
            &folder("top"),
            &[change(Subject::user("alice"), &["READ"], &[])],
        )
        .await
        .unwrap();
    store.rename(&folder("other"), "top", &[]).await.unwrap();
    assert_eq!(store.list(&folder("top")).await.unwrap().len(), 2);

    // delete: the object and its descendants of the given types.
    let deleted = store
        .delete_object(&folder("top"), &["Ns::Folder"])
        .await
        .unwrap();
    assert_eq!(deleted, 2);
    assert_eq!(store.list(&doc("top.a.d")).await.unwrap().len(), 1);
    let deleted = store.delete_object(&doc("top.a.d"), &[]).await.unwrap();
    assert_eq!(deleted, 1);
    assert!(store.list(&doc("top.a.d")).await.unwrap().is_empty());
}
