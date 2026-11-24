//! KEK rotation over a [`ManagedObjectStore`]: count which KEKs the stored blobs depend on, and
//! sweep every blob under a retired KEK onto the active one.
//!
//! See the [rotation steps](crate::encryption#kek-rotation) for how the two fit together.

use std::collections::BTreeMap;

use uuid::Uuid;

use super::ManagedObjectStore;
use crate::encryption::{KekId, envelope_kek_id};
use crate::label::Label;
use crate::store::{ObjectStore, ObjectStoreReader};
use crate::{Error, Result};

/// Page size [`ManagedObjectStore::kek_usage`] reads blobs in.
const USAGE_BATCH: usize = 256;

/// How many sealed blobs depend on each KEK, as counted by [`ManagedObjectStore::kek_usage`].
///
/// A KEK is safe to drop from the key provider once its count is zero **and**
/// [`unreadable`](Self::unreadable) is zero (an unreadable header could hide a blob under any
/// KEK).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KekUsage {
    /// Number of blobs per KEK id. KEKs with no blobs are absent.
    pub by_kek: BTreeMap<KekId, u64>,
    /// Number of blobs whose envelope header could not be read (not an envelope, or an
    /// unsupported format version).
    pub unreadable: u64,
}

impl KekUsage {
    /// Number of blobs sealed under `kek_id`.
    pub fn count(&self, kek_id: &str) -> u64 {
        self.by_kek.get(kek_id).copied().unwrap_or(0)
    }

    /// Total number of blobs counted, readable or not.
    pub fn total(&self) -> u64 {
        self.by_kek.values().sum::<u64>() + self.unreadable
    }

    fn record(&mut self, blob: &[u8]) {
        match envelope_kek_id(blob) {
            Ok(kek_id) => *self.by_kek.entry(kek_id).or_default() += 1,
            Err(_) => self.unreadable += 1,
        }
    }
}

/// Options for [`ManagedObjectStore::rewrap_all`].
#[derive(Debug, Clone, Copy)]
pub struct RewrapOptions<L> {
    /// How many blobs to read from the backend per page. Must be at least 1.
    pub batch_size: usize,
    /// Sweep only objects of this label. `None` sweeps every label in the registry.
    pub label: Option<L>,
}

impl<L> Default for RewrapOptions<L> {
    fn default() -> Self {
        Self {
            batch_size: USAGE_BATCH,
            label: None,
        }
    }
}

/// Why [`ManagedObjectStore::rewrap_all`] could not move one blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewrapFailureKind {
    /// The blob's envelope header could not be read.
    Malformed,
    /// The key provider could not unwrap the data key under the recorded KEK (for example, the
    /// KEK is no longer configured) or could not wrap it under the active one.
    KeyProvider,
    /// Re-reading or writing the blob failed in the backend. Carries the error's stable kind
    /// string (`"not_found"`, `"generic"`, …).
    Store(&'static str),
}

/// A blob that [`ManagedObjectStore::rewrap_all`] left where it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewrapFailure<L> {
    /// The object the blob belongs to.
    pub id: Uuid,
    /// The object's label.
    pub label: L,
    /// The KEK the blob is still under, if its header could be read.
    pub kek_id: Option<KekId>,
    /// What went wrong.
    pub kind: RewrapFailureKind,
}

/// The outcome of [`ManagedObjectStore::rewrap_all`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewrapReport<L> {
    /// Blobs moved from a retired KEK to the active one.
    pub rewrapped: u64,
    /// Blobs already under the active KEK, left untouched.
    pub already_active: u64,
    /// Blobs that could not be moved. The sweep carries on past each one.
    pub failed: Vec<RewrapFailure<L>>,
    /// KEK usage across **all** labels, counted after the sweep, whatever
    /// [`RewrapOptions::label`] was set to.
    pub kek_usage: KekUsage,
}

impl<L: Label, S: ObjectStoreReader<L>> ManagedObjectStore<L, S> {
    /// Count the sealed blobs per KEK, across every label in the registry.
    ///
    /// Reads the blobs in pages through
    /// [`list_sensitive`](ObjectStoreReader::list_sensitive) and peeks at each envelope header
    /// ([`envelope_kek_id`]), so it never decrypts anything, never holds more than one page in
    /// memory and works without an encryptor. Only labels known to the registry are counted.
    ///
    /// The count is a snapshot: a blob written while it runs may or may not be included.
    ///
    /// # Errors
    ///
    /// Returns any error the backend reports while listing blobs.
    #[tracing::instrument(skip_all, fields(otel.name = "olai_store.kek_usage"))]
    pub async fn kek_usage(&self) -> Result<KekUsage> {
        let mut usage = KekUsage::default();
        for &label in self.registry.labels() {
            let mut token = None;
            loop {
                let (page, next) = self
                    .inner
                    .list_sensitive(label, Some(USAGE_BATCH), token)
                    .await?;
                for (_, blob) in &page {
                    usage.record(blob);
                }
                match next {
                    Some(t) => token = Some(t),
                    None => break,
                }
            }
        }
        Ok(usage)
    }
}

impl<L: Label, S: ObjectStore<L>> ManagedObjectStore<L, S> {
    /// Re-wrap every sealed blob that is under a retired KEK onto the active KEK.
    ///
    /// This is the eager counterpart to the lazy rewrap in
    /// [`get_with_secrets`](crate::SecretObjectReader::get_with_secrets): it reaches blobs nobody
    /// reads. Blobs are read in pages of [`RewrapOptions::batch_size`]. For each one under a
    /// retired KEK the current blob is re-read, its data key is re-wrapped
    /// ([`EnvelopeEncryptor::rewrap`](crate::EnvelopeEncryptor::rewrap), so the value ciphertext
    /// is untouched) and the result is written back with
    /// [`set_sensitive`](ObjectStore::set_sensitive). Properties, versions and `updated_at` are
    /// never touched.
    ///
    /// A blob that cannot be moved (unreadable header, KEK missing from the provider, backend
    /// write error) is recorded in [`RewrapReport::failed`] and the sweep moves on. An object
    /// deleted while the sweep runs is skipped and not reported.
    ///
    /// The sweep is idempotent. If it is interrupted (the future dropped, or an `Err` from the
    /// backend while listing), run it again: blobs already moved count as
    /// [`already_active`](RewrapReport::already_active) and the rest are moved.
    ///
    /// The write is not a compare-and-swap. If another writer replaces a blob between the
    /// sweep's re-read and its write, the sweep's write wins and puts the older value back.
    /// The window is one rewrap per blob, but run the sweep at a quiet time if writes to sealed
    /// fields are frequent.
    ///
    /// # Errors
    ///
    /// - [`Error::InvalidArgument`] if the store has no encryptor or `batch_size` is 0.
    /// - Any error the backend reports while listing blobs; the sweep stops there.
    #[tracing::instrument(skip_all, fields(otel.name = "olai_store.rewrap_all"))]
    pub async fn rewrap_all(&self, opts: RewrapOptions<L>) -> Result<RewrapReport<L>> {
        let Some(encryptor) = self.encryptor.as_ref() else {
            return Err(Error::invalid_argument(
                "rewrap_all needs a store built with an encryptor",
            ));
        };
        if opts.batch_size == 0 {
            return Err(Error::invalid_argument("batch_size must be at least 1"));
        }
        let active = encryptor.active_kek_id();
        let labels: Vec<L> = match opts.label {
            Some(label) => vec![label],
            None => self.registry.labels().copied().collect(),
        };

        let mut report = RewrapReport {
            rewrapped: 0,
            already_active: 0,
            failed: Vec::new(),
            kek_usage: KekUsage::default(),
        };
        for label in labels {
            let mut token = None;
            loop {
                let (page, next) = self
                    .inner
                    .list_sensitive(label, Some(opts.batch_size), token)
                    .await?;
                for (id, blob) in page {
                    self.rewrap_one(label, id, &blob, &active, &mut report)
                        .await;
                }
                match next {
                    Some(t) => token = Some(t),
                    None => break,
                }
            }
        }
        report.kek_usage = self.kek_usage().await?;
        tracing::info!(
            rewrapped = report.rewrapped,
            already_active = report.already_active,
            failed = report.failed.len(),
            "KEK rotation sweep finished"
        );
        Ok(report)
    }

    /// Move one listed blob, recording the outcome in `report`.
    async fn rewrap_one(
        &self,
        label: L,
        id: Uuid,
        listed: &[u8],
        active: &str,
        report: &mut RewrapReport<L>,
    ) {
        let encryptor = self
            .encryptor
            .as_ref()
            .expect("rewrap_all checked for an encryptor");
        let fail = |kek_id: Option<KekId>, kind| RewrapFailure {
            id,
            label,
            kek_id,
            kind,
        };

        match envelope_kek_id(listed) {
            Ok(kek_id) if kek_id == active => {
                report.already_active += 1;
                return;
            }
            Ok(_) => {}
            Err(_) => {
                report.failed.push(fail(None, RewrapFailureKind::Malformed));
                return;
            }
        }

        // Re-read so the write is based on the current blob, not one from the start of the page.
        let current = match self.inner.get_sensitive(&id).await {
            Ok(Some(blob)) => blob,
            // Deleted since it was listed: nothing left to move.
            Ok(None) => return,
            Err(e) => {
                let kek_id = envelope_kek_id(listed).ok();
                report
                    .failed
                    .push(fail(kek_id, RewrapFailureKind::Store(e.kind_str())));
                return;
            }
        };
        let Ok(kek_id) = envelope_kek_id(&current) else {
            report.failed.push(fail(None, RewrapFailureKind::Malformed));
            return;
        };
        let rewrapped = match encryptor.rewrap(&current).await {
            Ok(Some(rewrapped)) => rewrapped,
            // Someone moved it in the meantime.
            Ok(None) => {
                report.already_active += 1;
                return;
            }
            Err(e) => {
                tracing::debug!(
                    id = %id,
                    kek_id = %kek_id,
                    error.type = e.kind_str(),
                    "rewrap failed; blob stays under its KEK"
                );
                report
                    .failed
                    .push(fail(Some(kek_id), RewrapFailureKind::KeyProvider));
                return;
            }
        };
        match self
            .inner
            .set_sensitive(&id, bytes::Bytes::from(rewrapped))
            .await
        {
            Ok(()) => report.rewrapped += 1,
            // Deleted between the re-read and the write.
            Err(Error::NotFound) => {}
            Err(e) => {
                tracing::debug!(
                    id = %id,
                    kek_id = %kek_id,
                    error.type = e.kind_str(),
                    "rewrap writeback failed; blob stays under its KEK"
                );
                report
                    .failed
                    .push(fail(Some(kek_id), RewrapFailureKind::Store(e.kind_str())));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use bytes::Bytes;

    use super::*;
    use crate::InMemoryStore;
    use crate::encryption::{EnvelopeEncryptor, LocalKeyProvider};
    use crate::managed::tests::{TestLabel, props, registry, rn};
    use crate::name::ResourceName;
    use crate::object::Object;
    use crate::store::{Precondition, SecretObjectReader};

    fn key(byte: u8) -> Vec<u8> {
        vec![byte; 32]
    }

    /// An encryptor with `active` as the active KEK, holding every KEK in `held` (`vN` is
    /// `key(N)`).
    fn encryptor(active: &str, held: &[&str]) -> EnvelopeEncryptor {
        let keys = held.iter().map(|id| {
            let n: u8 = id[1..].parse().unwrap();
            ((*id).to_string(), key(n))
        });
        EnvelopeEncryptor::local(LocalKeyProvider::new(active, keys).unwrap())
    }

    fn store_with<S: ObjectStore<TestLabel>>(
        inner: S,
        active: &str,
        held: &[&str],
    ) -> ManagedObjectStore<TestLabel, S> {
        ManagedObjectStore::with_encryptor(inner, encryptor(active, held), registry())
    }

    /// Seal `count` widgets under KEK `kek` into `backend`, returning their ids.
    async fn seed(
        backend: &InMemoryStore<TestLabel>,
        kek: &str,
        prefix: &str,
        count: usize,
    ) -> Vec<Uuid> {
        let store = store_with(backend.clone(), kek, &[kek]);
        let mut ids = Vec::new();
        for i in 0..count {
            let obj = store
                .create(
                    TestLabel::Widget,
                    &rn(&format!("{prefix}{i}")),
                    props(
                        serde_json::json!({ "color": "red", "api_key": format!("{prefix}-{i}") }),
                    ),
                    None,
                    None,
                )
                .await
                .unwrap();
            ids.push(obj.id);
        }
        ids
    }

    fn usage(pairs: &[(&str, u64)], unreadable: u64) -> KekUsage {
        KekUsage {
            by_kek: pairs.iter().map(|(k, n)| ((*k).to_string(), *n)).collect(),
            unreadable,
        }
    }

    #[tokio::test]
    async fn sweep_moves_every_retired_blob() {
        let backend = InMemoryStore::<TestLabel>::new();
        let v1 = seed(&backend, "v1", "a", 5).await;
        let v2 = seed(&backend, "v2", "b", 3).await;
        let v3 = seed(&backend, "v3", "c", 2).await;
        // An object with no sensitive fields carries no blob and is never counted.
        store_with(backend.clone(), "v3", &["v3"])
            .create(
                TestLabel::Other,
                &rn("plain"),
                props(serde_json::json!({ "value": 1 })),
                None,
                None,
            )
            .await
            .unwrap();

        let store = store_with(backend.clone(), "v3", &["v1", "v2", "v3"]);
        assert_eq!(
            store.kek_usage().await.unwrap(),
            usage(&[("v1", 5), ("v2", 3), ("v3", 2)], 0)
        );

        let before: Vec<Object<TestLabel>> = {
            let mut out = Vec::new();
            for id in v1.iter().chain(&v2).chain(&v3) {
                out.push(backend.get(id).await.unwrap());
            }
            out
        };

        // A batch size smaller than the population forces several pages.
        let report = store
            .rewrap_all(RewrapOptions {
                batch_size: 2,
                label: None,
            })
            .await
            .unwrap();
        assert_eq!(report.rewrapped, 8);
        assert_eq!(report.already_active, 2);
        assert!(report.failed.is_empty(), "{:?}", report.failed);
        assert_eq!(report.kek_usage, usage(&[("v3", 10)], 0));
        assert_eq!(store.kek_usage().await.unwrap(), report.kek_usage);

        // Properties, versions and timestamps are untouched; only the blob moved.
        for old in &before {
            let now = backend.get(&old.id).await.unwrap();
            assert_eq!(&now, old, "the sweep must not touch the object row");
        }

        // A store holding only the active KEK opens every value.
        let v3_only = store_with(backend.clone(), "v3", &["v3"]);
        for (i, id) in v1.iter().enumerate() {
            let full = v3_only.get_with_secrets(id).await.unwrap();
            assert_eq!(
                full.properties.unwrap()["api_key"],
                serde_json::json!(format!("a-{i}"))
            );
        }

        // Running it again finds nothing to do.
        let again = store.rewrap_all(RewrapOptions::default()).await.unwrap();
        assert_eq!((again.rewrapped, again.already_active), (0, 10));
    }

    #[tokio::test]
    async fn missing_kek_is_reported_not_fatal() {
        let backend = InMemoryStore::<TestLabel>::new();
        let lost = seed(&backend, "v0", "lost", 1).await[0];
        seed(&backend, "v1", "a", 3).await;

        // `v0` is not configured any more: its blob cannot be moved.
        let store = store_with(backend.clone(), "v2", &["v1", "v2"]);
        let report = store
            .rewrap_all(RewrapOptions {
                batch_size: 1,
                label: None,
            })
            .await
            .unwrap();
        assert_eq!(report.rewrapped, 3);
        assert_eq!(
            report.failed,
            vec![RewrapFailure {
                id: lost,
                label: TestLabel::Widget,
                kek_id: Some("v0".into()),
                kind: RewrapFailureKind::KeyProvider,
            }]
        );
        // The count still shows the stranded blob, so `v0` is visibly not safe to forget.
        assert_eq!(report.kek_usage, usage(&[("v0", 1), ("v2", 3)], 0));
    }

    #[tokio::test]
    async fn unreadable_blob_is_reported_and_counted() {
        let backend = InMemoryStore::<TestLabel>::new();
        seed(&backend, "v1", "a", 2).await;
        let junk = backend
            .create(
                TestLabel::Widget,
                &rn("junk"),
                None,
                None,
                Some(Bytes::from_static(b"\x09junk")),
            )
            .await
            .unwrap();

        let store = store_with(backend.clone(), "v2", &["v1", "v2"]);
        assert_eq!(store.kek_usage().await.unwrap(), usage(&[("v1", 2)], 1));

        let report = store.rewrap_all(RewrapOptions::default()).await.unwrap();
        assert_eq!(report.rewrapped, 2);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].id, junk.id);
        assert_eq!(report.failed[0].kind, RewrapFailureKind::Malformed);
        assert_eq!(report.kek_usage, usage(&[("v2", 2)], 1));
        assert_eq!(report.kek_usage.total(), 3);
    }

    #[tokio::test]
    async fn label_filter_sweeps_one_label_but_counts_all() {
        static FIELDS: &[crate::registry::ResourceFieldDescriptor] =
            &[crate::registry::ResourceFieldDescriptor {
                name: "token",
                role: crate::registry::FieldRole::Sensitive,
            }];
        static DESCRIPTORS: &[crate::registry::ResourceTypeDescriptor<TestLabel>] = &[
            crate::registry::ResourceTypeDescriptor {
                label: TestLabel::Widget,
                fields: FIELDS,
                path_names: &["name"],
                parent_label: None,
            },
            crate::registry::ResourceTypeDescriptor {
                label: TestLabel::Other,
                fields: FIELDS,
                path_names: &["name"],
                parent_label: None,
            },
        ];
        let both = || crate::ResourceRegistry::from_static(DESCRIPTORS);

        let backend = InMemoryStore::<TestLabel>::new();
        let v1 =
            ManagedObjectStore::with_encryptor(backend.clone(), encryptor("v1", &["v1"]), both());
        for (label, name) in [(TestLabel::Widget, "w"), (TestLabel::Other, "o")] {
            v1.create(
                label,
                &rn(name),
                props(serde_json::json!({ "token": "t" })),
                None,
                None,
            )
            .await
            .unwrap();
        }

        let store =
            ManagedObjectStore::with_encryptor(backend, encryptor("v2", &["v1", "v2"]), both());
        let report = store
            .rewrap_all(RewrapOptions {
                batch_size: 10,
                label: Some(TestLabel::Widget),
            })
            .await
            .unwrap();
        assert_eq!(report.rewrapped, 1);
        assert_eq!(report.kek_usage, usage(&[("v1", 1), ("v2", 1)], 0));
    }

    #[tokio::test]
    async fn rejects_missing_encryptor_and_zero_batch() {
        let backend = InMemoryStore::<TestLabel>::new();
        seed(&backend, "v1", "a", 1).await;

        let plain = ManagedObjectStore::new(backend.clone(), registry());
        let err = plain
            .rewrap_all(RewrapOptions::default())
            .await
            .unwrap_err();
        assert!(matches!(err, Error::InvalidArgument(_)), "{err:?}");
        // Counting needs no encryptor.
        assert_eq!(plain.kek_usage().await.unwrap(), usage(&[("v1", 1)], 0));

        let store = store_with(backend, "v1", &["v1"]);
        let err = store
            .rewrap_all(RewrapOptions {
                batch_size: 0,
                label: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, Error::InvalidArgument(_)), "{err:?}");
    }

    /// A backend that forwards to an [`InMemoryStore`], except that `list_sensitive` fails once
    /// its budget of successful calls is spent, and `set_sensitive` can be made to fail.
    struct Flaky {
        inner: InMemoryStore<TestLabel>,
        list_budget: AtomicUsize,
        fail_writes: bool,
    }

    impl Flaky {
        fn new(inner: InMemoryStore<TestLabel>, list_budget: usize) -> Self {
            Self {
                inner,
                list_budget: AtomicUsize::new(list_budget),
                fail_writes: false,
            }
        }
    }

    #[async_trait::async_trait]
    impl ObjectStoreReader<TestLabel> for Flaky {
        async fn get(&self, id: &Uuid) -> Result<Object<TestLabel>> {
            self.inner.get(id).await
        }
        async fn get_by_name(
            &self,
            label: TestLabel,
            name: &ResourceName,
        ) -> Result<Object<TestLabel>> {
            self.inner.get_by_name(label, name).await
        }
        async fn list(
            &self,
            label: TestLabel,
            namespace: Option<&ResourceName>,
            max_results: Option<usize>,
            page_token: Option<String>,
        ) -> Result<(Vec<Object<TestLabel>>, Option<String>)> {
            self.inner
                .list(label, namespace, max_results, page_token)
                .await
        }
        async fn get_sensitive(&self, id: &Uuid) -> Result<Option<Bytes>> {
            self.inner.get_sensitive(id).await
        }
        async fn list_sensitive(
            &self,
            label: TestLabel,
            max_results: Option<usize>,
            page_token: Option<String>,
        ) -> Result<crate::store::SensitivePage> {
            // Single-threaded tests: a load then a decrement is enough.
            if self.list_budget.load(Ordering::SeqCst) == 0 {
                return Err(Error::generic("connection lost"));
            }
            self.list_budget.fetch_sub(1, Ordering::SeqCst);
            self.inner
                .list_sensitive(label, max_results, page_token)
                .await
        }
    }

    #[async_trait::async_trait]
    impl ObjectStore<TestLabel> for Flaky {
        async fn create(
            &self,
            label: TestLabel,
            name: &ResourceName,
            properties: Option<serde_json::Value>,
            id: Option<Uuid>,
            sensitive: Option<Bytes>,
        ) -> Result<Object<TestLabel>> {
            self.inner
                .create(label, name, properties, id, sensitive)
                .await
        }
        async fn update(
            &self,
            id: &Uuid,
            properties: Option<serde_json::Value>,
            precondition: Precondition,
            sensitive: Option<Bytes>,
        ) -> Result<Object<TestLabel>> {
            self.inner
                .update(id, properties, precondition, sensitive)
                .await
        }
        async fn rename(
            &self,
            id: &Uuid,
            new_name: &ResourceName,
            precondition: Precondition,
        ) -> Result<Object<TestLabel>> {
            self.inner.rename(id, new_name, precondition).await
        }
        async fn delete(&self, id: &Uuid) -> Result<()> {
            self.inner.delete(id).await
        }
        async fn set_sensitive(&self, id: &Uuid, sensitive: Bytes) -> Result<()> {
            if self.fail_writes {
                return Err(Error::generic("read-only replica"));
            }
            self.inner.set_sensitive(id, sensitive).await
        }
    }

    #[tokio::test]
    async fn interrupted_sweep_resumes() {
        let backend = InMemoryStore::<TestLabel>::new();
        seed(&backend, "v1", "a", 7).await;

        // The backend drops out after two pages of three: the sweep stops with the error...
        let flaky = store_with(Flaky::new(backend.clone(), 2), "v2", &["v1", "v2"]);
        let err = flaky
            .rewrap_all(RewrapOptions {
                batch_size: 3,
                label: Some(TestLabel::Widget),
            })
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Generic(_)), "{err:?}");

        // ...having moved the blobs it had reached.
        let store = store_with(backend.clone(), "v2", &["v1", "v2"]);
        assert_eq!(
            store.kek_usage().await.unwrap(),
            usage(&[("v1", 1), ("v2", 6)], 0)
        );

        // Running it again moves the rest and counts the earlier work as already done.
        let report = store
            .rewrap_all(RewrapOptions {
                batch_size: 3,
                label: None,
            })
            .await
            .unwrap();
        assert_eq!((report.rewrapped, report.already_active), (1, 6));
        assert!(report.failed.is_empty());
        assert_eq!(report.kek_usage, usage(&[("v2", 7)], 0));
    }

    #[tokio::test]
    async fn write_failure_is_reported_per_blob() {
        let backend = InMemoryStore::<TestLabel>::new();
        let ids = seed(&backend, "v1", "a", 2).await;

        let mut flaky = Flaky::new(backend, usize::MAX);
        flaky.fail_writes = true;
        let store = store_with(flaky, "v2", &["v1", "v2"]);
        let report = store.rewrap_all(RewrapOptions::default()).await.unwrap();
        assert_eq!(report.rewrapped, 0);
        let failed: Vec<_> = report.failed.iter().map(|f| (f.id, f.kind)).collect();
        assert_eq!(
            failed,
            ids.iter()
                .map(|id| (*id, RewrapFailureKind::Store("generic")))
                .collect::<Vec<_>>()
        );
        assert_eq!(report.kek_usage, usage(&[("v1", 2)], 0));
    }
}
