//! Two processes on one store: the test re-runs its own binary as a child writer.

use std::process::Command;
use std::time::Duration;

use olai_stack_state::{Draft, Store};
use olai_stack_topology::{EnvManifest, PlanCtx, Selection};

/// Set in the child: the database to write to, and how many instances to lease.
const CHILD_DB: &str = "OLAI_STACK_STATE_CHILD_DB";

fn draft() -> Draft {
    Draft::new(EnvManifest::new(Selection::default(), PlanCtx::default()))
}

/// Create a workspace with `count` instances and lease each a slot.
fn lease_many(store: &Store, workspace: &str, count: usize) -> Vec<u16> {
    let ws = store.create_workspace(workspace, false).unwrap();
    let env = store.create_environment(&ws.id, "dev", draft()).unwrap();
    store.commit_draft(&env.id).unwrap();
    (0..count)
        .map(|_| {
            let instance = store.create_instance(&env.id, 1).unwrap();
            store.lease_slot(&instance.id).unwrap().slot
        })
        .collect()
}

/// The child's half; a no-op unless the parent started this binary with [`CHILD_DB`].
#[test]
fn child_writer() {
    let Some(path) = std::env::var_os(CHILD_DB) else {
        return;
    };
    let store = Store::open(std::path::Path::new(&path)).unwrap();
    let slots = lease_many(&store, "child", 20);
    println!("CHILD_SLOTS={slots:?}");
}

#[tokio::test]
async fn two_processes_write_concurrently_and_see_each_other() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let store = Store::open(&path).unwrap();
    let mut watcher = store.watcher().unwrap();
    assert!(!watcher.changed().unwrap());

    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_writer", "--nocapture", "--test-threads=1"])
        .env(CHILD_DB, &path)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // Write while the child does.
    let mine = lease_many(&store, "parent", 20);
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );

    // Every slot was leased once, across both processes.
    let leases = store.leases().unwrap();
    assert_eq!(leases.len(), 40);
    let mut slots: Vec<u16> = leases.iter().map(|l| l.slot).collect();
    slots.dedup();
    assert_eq!(slots.len(), 40);
    assert!(mine.iter().all(|s| slots.contains(s)));
    assert_eq!(store.workspaces().unwrap().len(), 2);

    // The child's commits show up in the change feed; this process's own don't count.
    assert!(watcher.changed().unwrap());
    assert!(!watcher.changed().unwrap());

    // The async feed yields after the next outside write.
    let feed = store.watcher().unwrap().changes(Duration::from_millis(20));
    let writer = tokio::task::spawn_blocking(move || {
        std::thread::sleep(Duration::from_millis(100));
        let other = Store::open(&path).unwrap();
        other.create_workspace("late", false).unwrap();
    });
    let mut feed = Box::pin(feed);
    let next = tokio::time::timeout(Duration::from_secs(5), futures::StreamExt::next(&mut feed))
        .await
        .expect("a change within 5s");
    assert!(matches!(next, Some(Ok(()))));
    writer.await.unwrap();
}
