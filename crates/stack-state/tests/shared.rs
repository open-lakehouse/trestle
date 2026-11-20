//! What sharing a store adds: draft conflicts, attachments, and, on Postgres, two machines on
//! one team database.

mod common;

use common::{Harness, draft};
use olai_stack_state::{Error, RemoteLocation, Store};

fn neon(project: &str) -> RemoteLocation {
    RemoteLocation::Neon {
        org: None,
        project: project.into(),
        branch: Some("main".into()),
        database: "trestle_state".into(),
    }
}

fn a_stale_draft_edit_is_refused(h: &Harness) {
    let (mine, theirs) = (h.store(), h.store());
    let ws = mine.create_workspace("demo", false).unwrap();
    let env = mine.create_environment(&ws.id, "dev", draft(&[])).unwrap();
    assert_eq!(env.draft_version, 0);

    // Both read version 0; they write first.
    let read = theirs.environment(&env.id).unwrap();
    let version = theirs
        .update_draft_if(&env.id, &draft(&["mlflow"]), read.draft_version)
        .unwrap();
    assert_eq!(version, 1);
    match mine.update_draft_if(&env.id, &draft(&["jaeger"]), env.draft_version) {
        Err(Error::Conflict {
            name,
            expected: 0,
            current: 1,
        }) => assert_eq!(name, "dev"),
        other => panic!("expected a conflict, got {other:?}"),
    }
    // Their draft stands; after reloading, mine goes through.
    let reloaded = mine.environment(&env.id).unwrap();
    assert_eq!(reloaded.draft.manifest.selection.modules.len(), 1);
    assert_eq!(
        mine.update_draft_if(&env.id, &draft(&["jaeger"]), reloaded.draft_version)
            .unwrap(),
        2
    );
    // Any edit of the draft moves the version: a rename too.
    mine.rename_environment(&env.id, "staging").unwrap();
    let env = mine.environment(&env.id).unwrap();
    assert_eq!(env.draft_version, 3);
    assert_eq!(env.draft.manifest.context.env_name, "staging");
    assert!(matches!(
        mine.update_draft_if(&env.id, &draft(&[]), 2),
        Err(Error::Conflict { .. })
    ));
    // The kept name wins over the draft's.
    let mut renamed = draft(&[]);
    renamed.manifest.context.env_name = "other".into();
    mine.update_draft_if(&env.id, &renamed, 3).unwrap();
    assert_eq!(
        mine.environment(&env.id)
            .unwrap()
            .draft
            .manifest
            .context
            .env_name,
        "staging"
    );
}

fn concurrent_conditional_edits_let_exactly_one_win(h: &Harness) {
    let store = h.store();
    let ws = store.create_workspace("demo", false).unwrap();
    let env = store.create_environment(&ws.id, "dev", draft(&[])).unwrap();
    let results: Vec<_> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let id = env.id.clone();
                scope.spawn(move || h.store().update_draft_if(&id, &draft(&["mlflow"]), 0))
            })
            .collect();
        handles.into_iter().map(|t| t.join().unwrap()).collect()
    });
    assert_eq!(
        results.iter().filter(|r| r.is_ok()).count(),
        1,
        "{results:?}"
    );
    assert!(
        results
            .iter()
            .filter(|r| r.is_err())
            .all(|r| matches!(r, Err(Error::Conflict { current: 1, .. })))
    );
}

on_both!(
    a_stale_draft_edit_is_refused,
    concurrent_conditional_edits_let_exactly_one_win,
);

#[test]
fn attachments_are_recorded_per_machine() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state.db")).unwrap();
    assert!(store.attachment_id().is_none());
    let team = store.attach("team", neon("p-1")).unwrap();
    assert_eq!(store.attachment(&team.id).unwrap(), team);
    assert_eq!(store.attachments().unwrap(), std::slice::from_ref(&team));

    // A name and a location are attached once.
    assert!(matches!(
        store.attach("team", neon("p-2")),
        Err(Error::Duplicate { kind: "attachment", name }) if name == "team"
    ));
    assert!(matches!(
        store.attach("other", neon("p-1")),
        Err(Error::Duplicate { kind: "attachment", name }) if name == "neon:p-1@main/trestle_state"
    ));
    assert!(matches!(
        store.attach("Bad", neon("p-3")),
        Err(Error::InvalidName(_))
    ));

    store.detach(&team.id).unwrap();
    assert!(store.attachments().unwrap().is_empty());
    assert!(matches!(
        store.detach(&team.id),
        Err(Error::NotFound {
            kind: "attachment",
            ..
        })
    ));
}

#[test]
fn a_location_round_trips_as_json() {
    let location = RemoteLocation::Neon {
        org: Some("org-1".into()),
        project: "p-1".into(),
        branch: None,
        database: "trestle_state".into(),
    };
    let text = serde_json::to_string(&location).unwrap();
    assert_eq!(
        text,
        r#"{"neon":{"org":"org-1","project":"p-1","database":"trestle_state"}}"#
    );
    assert_eq!(
        serde_json::from_str::<RemoteLocation>(&text).unwrap(),
        location
    );
    assert_eq!(location.to_string(), "neon:org-1/p-1/trestle_state");
}

#[cfg(feature = "postgres")]
mod on_postgres {
    use std::time::Duration;

    use olai_stack_state::{REMOTE_SCHEMA_VERSION, SecretRef, VarScope, VarValue};

    use super::*;

    #[test]
    fn two_machines_share_environments_but_not_instances_or_local_vars() {
        let Some(h) = Harness::postgres() else { return };
        let other_dir = tempfile::tempdir().unwrap();
        let (mine, theirs) = (h.store(), h.store_at(&other_dir.path().join("state.db")));
        assert!(mine.attachment_id().is_some());
        assert_ne!(mine.attachment_id(), theirs.attachment_id());

        let ws = mine.create_workspace("team", true).unwrap();
        let env = mine
            .create_environment(&ws.id, "dev", draft(&["unity-catalog-oss"]))
            .unwrap();
        let r1 = mine.commit_draft(&env.id).unwrap();
        assert_eq!(theirs.revision(&env.id, 1).unwrap(), r1);

        // Each runs an instance; each sees only its own, which names the attachment.
        let my_instance = mine.create_instance(&env.id, 1).unwrap();
        assert_eq!(my_instance.store.as_ref(), mine.attachment_id());
        mine.lease_slot(&my_instance.id).unwrap();
        let their_instance = theirs.create_instance(&env.id, 1).unwrap();
        assert_eq!(
            mine.instances_of(&env.id).unwrap(),
            [mine.instance(&my_instance.id).unwrap()]
        );
        assert_eq!(theirs.instances().unwrap().len(), 1);
        // The local store on my machine lists my instance too, but not the team's
        // workspaces.
        let local = Store::open(&h.path()).unwrap();
        assert_eq!(local.instances().unwrap().len(), 1);
        assert!(local.workspaces().unwrap().is_empty());

        // My machine's own key stays on my machine.
        let key = VarValue::Secret(SecretRef {
            provider: "keychain".into(),
            key: format!("{}/UC_SERVER_KEK", env.id),
        });
        mine.modify_vars(&VarScope::Local(env.id.clone()), |vars| {
            vars.set("UC_SERVER_KEK", key.clone());
            Ok::<_, Error>(())
        })
        .unwrap();
        assert_eq!(
            mine.local_vars(&env.id).unwrap().get("UC_SERVER_KEK"),
            Some(&key)
        );
        assert!(theirs.local_vars(&env.id).unwrap().is_empty());
        assert!(theirs.environment(&env.id).unwrap().vars.is_empty());

        // Only this machine's instances hold a rename back.
        assert!(matches!(
            theirs.rename_environment(&env.id, "x"),
            Err(Error::HasInstances { instances: 1, .. })
        ));
        theirs.delete_instance(&their_instance.id).unwrap();
        assert!(matches!(
            mine.rename_environment(&env.id, "x"),
            Err(Error::HasInstances { instances: 1, .. })
        ));

        // My instance holds the attachment; deleting the environment drops what my machine
        // keeps of it.
        assert!(matches!(
            local.detach(mine.attachment_id().unwrap()),
            Err(Error::HasInstances {
                kind: "attachment",
                ..
            })
        ));
        mine.delete_environment(&env.id).unwrap();
        assert!(mine.instances().unwrap().is_empty());
        assert!(mine.leases().unwrap().is_empty());
        assert!(mine.local_vars(&env.id).unwrap().is_empty());
        local.detach(mine.attachment_id().unwrap()).unwrap();
    }

    #[tokio::test]
    async fn a_commit_on_one_machine_shows_up_in_the_others_watcher() {
        let Some(h) = Harness::postgres() else { return };
        let other_dir = tempfile::tempdir().unwrap();
        let mine = h.store();
        let theirs = h.store_at(&other_dir.path().join("state.db"));
        let mut watcher = theirs.watcher().unwrap();
        assert!(!watcher.changed().unwrap());

        let ws = mine.create_workspace("team", false).unwrap();
        assert!(watcher.changed().unwrap());
        assert!(!watcher.changed().unwrap());
        let env = mine.create_environment(&ws.id, "dev", draft(&[])).unwrap();
        mine.commit_draft(&env.id).unwrap();
        assert!(watcher.changed().unwrap());

        // The feed yields after the next write, from a blocking task as the CLI's would be.
        let feed = theirs.watcher().unwrap().changes(Duration::from_millis(20));
        let writer = tokio::task::spawn_blocking(move || {
            std::thread::sleep(Duration::from_millis(100));
            mine.update_draft(&env.id, &draft(&["mlflow"])).unwrap();
        });
        let mut feed = Box::pin(feed);
        let next =
            tokio::time::timeout(Duration::from_secs(5), futures::StreamExt::next(&mut feed))
                .await
                .expect("a change within 5s");
        assert!(matches!(next, Some(Ok(()))));
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn the_store_works_from_inside_a_runtime() {
        // The desktop app and stack-runtime call the sync API on runtime threads.
        let Some(h) = Harness::postgres() else { return };
        let store = h.store();
        let ws = store.create_workspace("inside", false).unwrap();
        assert_eq!(store.workspace("inside").unwrap(), Some(ws));
    }

    #[test]
    fn opening_checks_the_schema_and_never_migrates_it() {
        let Some(h) = Harness::postgres() else { return };
        let url = h.url().unwrap().to_owned();
        let attachment = h.store().attachment_id().unwrap().clone();
        let open = || {
            let url = url.clone();
            Store::open_attached(&h.path(), &attachment, move |_| {
                Ok::<_, std::convert::Infallible>(url)
            })
        };

        h.raw(&format!(
            "UPDATE state_meta SET value = {} WHERE key = 'schema_version'",
            REMOTE_SCHEMA_VERSION + 1
        ))
        .unwrap();
        assert!(matches!(
            open(),
            Err(Error::UnsupportedVersion { found, supported })
                if found == REMOTE_SCHEMA_VERSION + 1 && supported == REMOTE_SCHEMA_VERSION
        ));
        assert!(matches!(
            Store::init_remote(&url),
            Err(Error::UnsupportedVersion { .. })
        ));

        h.raw("UPDATE state_meta SET value = 0 WHERE key = 'schema_version'")
            .unwrap();
        assert!(matches!(open(), Err(Error::Outdated { found: 0, .. })));

        h.raw("DROP TABLE state_meta").unwrap();
        assert!(matches!(open(), Err(Error::Uninitialized)));
    }

    #[test]
    fn initializing_twice_is_harmless() {
        let Some(h) = Harness::postgres() else { return };
        let store = h.store();
        store.create_workspace("kept", false).unwrap();
        Store::init_remote(h.url().unwrap()).unwrap();
        assert!(store.workspace("kept").unwrap().is_some());
    }

    #[test]
    fn a_resolver_error_and_a_bad_url_are_reported() {
        let Some(h) = Harness::postgres() else { return };
        let attachment = h.store().attachment_id().unwrap().clone();
        let err = Store::open_attached(&h.path(), &attachment, |location| {
            Err(format!("not signed in to {location}"))
        })
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "shared state store: not signed in to neon:test/trestle_state"
        );
        assert!(matches!(
            Store::open_attached(&h.path(), &attachment, |_| {
                Ok::<_, std::convert::Infallible>("not a url".into())
            }),
            Err(Error::Remote(_))
        ));
        let unknown = olai_stack_state::AttachmentId("nope".into());
        assert!(matches!(
            Store::open_attached(&h.path(), &unknown, |_| Ok::<_, std::convert::Infallible>(
                String::new()
            )),
            Err(Error::NotFound {
                kind: "attachment",
                ..
            })
        ));
    }
}
