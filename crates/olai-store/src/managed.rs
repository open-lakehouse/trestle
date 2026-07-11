//! Registry-aware object store decorator that enforces field roles.
//!
//! [`ManagedObjectStore`] wraps an [`ObjectStore`] and uses a [`ResourceRegistry`] to
//! automatically:
//!
//! - Strip [`FieldRole::Identifier`] and [`FieldRole::Managed`] fields on create/update
//!   (the store is the source of truth for these)
//! - Route [`FieldRole::Sensitive`] fields into an envelope-encrypted blob stored *inline* on
//!   the object row (see below), with the `encryption` feature
//! - Inject Identifier and Managed fields back into properties on read
//! - Redact Sensitive fields on read (unless [`get_with_secrets`] is used)
//!
//! # Sensitive fields
//!
//! Sensitive fields (proto `debug_redact = true`) are split out of the object's `properties`
//! and, when an `EnvelopeEncryptor` is configured, sealed into an opaque blob that is written
//! *atomically with the object* through the store's
//! [`sensitive`](ObjectStore::create) parameter. Because the sealed blob rides the same row,
//! there is no separate secret store and no window in which an object exists without its secret
//! (or vice versa): create/update/delete are single atomic writes.
//!
//! The blob is bound (as AEAD associated data) to the object's UUID, so a sealed value cannot be
//! relocated to a different object. A store built without an encryptor cannot store sensitive
//! fields at all: writing a resource that has one is an [`Error::InvalidArgument`], never a silent
//! drop.
//!
//! Because sensitive fields never enter `properties`, they are absent from the searchable payload:
//! encrypting them does not reduce searchability, and there is no need for (and this crate does
//! not provide) searchable encryption.
//!
//! # Searchability of managed fields
//!
//! [`search`](crate::ObjectStoreReader::search) filters the *stored* `properties` — the plaintext
//! data fields. Store-owned `Identifier` and `Managed` fields are stripped on write and only
//! injected back on read, so, like `Sensitive` fields, they are **not part of the searchable
//! payload**. Filter on data fields; address a resource by its identifier with
//! [`get_by_name`](crate::ObjectStoreReader::get_by_name). A resource's `created_at`/`updated_at`
//! are first-class [`Object`] columns rather than payload, and time-range queries
//! over associations are a separate, deferred concern (they need a `created_at` index and keyset
//! pagination, not the payload filter).
//!
//! [`get_with_secrets`]: ManagedObjectStore::get_with_secrets

use std::marker::PhantomData;
use std::sync::Arc;

use uuid::Uuid;

use crate::label::Label;
use crate::name::ResourceName;
use crate::object::Object;
use crate::registry::{FieldRole, ResourceRegistry};
use crate::store::{ObjectStore, ObjectStoreReader, Precondition};
use crate::{Error, Result};

/// A registry-aware object store that enforces field roles — the primary store API.
///
/// Wraps an inner [`ObjectStore`] backend (the taxonomy-blind blob layer, e.g.
/// [`InMemoryStore`](crate::InMemoryStore) or `SqlStore`) and uses a [`ResourceRegistry`] to
/// determine how each field is handled during CRUD: store-owned `Identifier`/`Managed` fields are
/// stripped on write and injected on read, and `Sensitive` fields (proto `debug_redact = true`)
/// are redacted on read.
///
/// When an `EnvelopeEncryptor` is provided (via
/// `with_encryptor`, behind the `encryption` feature),
/// sensitive fields are sealed into an opaque blob stored inline on the object row, written
/// atomically with the object; [`get_with_secrets`](ManagedObjectStore::get_with_secrets) opens
/// them back.
///
/// **A resource that has sensitive fields can only be written through a store that can encrypt
/// them.** Attempting to `create`/`update` such a resource on a store built with
/// [`new`](ManagedObjectStore::new) (no encryptor) is an [`Error::InvalidArgument`] — the crate
/// never silently drops secret data. Resources with no sensitive fields need no encryptor.
pub struct ManagedObjectStore<L: Label, S> {
    inner: S,
    #[cfg(feature = "encryption")]
    encryptor: Option<crate::encryption::EnvelopeEncryptor>,
    registry: Arc<ResourceRegistry<L>>,
    _label: PhantomData<L>,
}

impl<L: Label, S: ObjectStore<L>> ManagedObjectStore<L, S> {
    /// Create a managed store without encryption.
    ///
    /// Suitable for taxonomies with **no** sensitive fields. Writing a resource that has a
    /// sensitive field through such a store is an [`Error::InvalidArgument`] (see the type-level
    /// docs); use `with_encryptor` (the `encryption` feature) for those.
    pub fn new(inner: S, registry: ResourceRegistry<L>) -> Self {
        Self {
            inner,
            #[cfg(feature = "encryption")]
            encryptor: None,
            registry: Arc::new(registry),
            _label: PhantomData,
        }
    }

    /// Create a managed store that seals sensitive fields with `encryptor`.
    ///
    /// Sensitive fields are sealed into an opaque blob stored inline on the object row, written
    /// atomically with the object.
    #[cfg(feature = "encryption")]
    pub fn with_encryptor(
        inner: S,
        encryptor: crate::encryption::EnvelopeEncryptor,
        registry: ResourceRegistry<L>,
    ) -> Self {
        Self {
            inner,
            encryptor: Some(encryptor),
            registry: Arc::new(registry),
            _label: PhantomData,
        }
    }
}

impl<L: Label, S> ManagedObjectStore<L, S> {
    /// Strip fields that should not be stored in properties on create/update.
    ///
    /// Returns `(stripped_properties, sensitive_fields_map)`.
    fn strip_fields(
        &self,
        label: L,
        properties: Option<serde_json::Value>,
    ) -> (
        Option<serde_json::Value>,
        Option<serde_json::Map<String, serde_json::Value>>,
    ) {
        let Some(serde_json::Value::Object(mut map)) = properties else {
            return (properties, None);
        };

        let Some(descriptor) = self.registry.get(label) else {
            return (Some(serde_json::Value::Object(map)), None);
        };

        let mut sensitive_map = serde_json::Map::new();

        for field in descriptor.fields.iter() {
            match field.role {
                FieldRole::Identifier | FieldRole::Managed => {
                    // Remove — store manages these
                    map.remove(field.name);
                }
                FieldRole::Sensitive => {
                    // Extract — will be sealed and stored inline
                    if let Some(value) = map.remove(field.name) {
                        sensitive_map.insert(field.name.to_string(), value);
                    }
                }
                FieldRole::Data => {
                    // Keep as-is
                }
            }
        }

        let sensitive = if sensitive_map.is_empty() {
            None
        } else {
            Some(sensitive_map)
        };

        (Some(serde_json::Value::Object(map)), sensitive)
    }

    /// Inject Identifier and Managed fields from Object metadata into properties.
    fn inject_fields(&self, object: &mut Object<L>) {
        let Some(descriptor) = self.registry.get(object.label) else {
            return;
        };

        let map = object
            .properties
            .get_or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));

        let Some(map) = map.as_object_mut() else {
            return;
        };

        for field in descriptor.fields.iter() {
            match field.role {
                FieldRole::Identifier => {
                    map.insert(
                        field.name.to_string(),
                        serde_json::Value::String(object.id.to_string()),
                    );
                }
                FieldRole::Managed => {
                    match field.name {
                        "created_at" => {
                            map.insert(
                                field.name.to_string(),
                                serde_json::Value::String(object.created_at.to_rfc3339()),
                            );
                        }
                        "updated_at" => {
                            if let Some(updated) = object.updated_at {
                                map.insert(
                                    field.name.to_string(),
                                    serde_json::Value::String(updated.to_rfc3339()),
                                );
                            }
                        }
                        _ => {
                            // Other managed fields (created_by, updated_by) — leave as-is
                            // if already present, don't overwrite
                        }
                    }
                }
                FieldRole::Sensitive => {
                    // Redact: ensure sensitive fields are absent from the response
                    map.remove(field.name);
                }
                FieldRole::Data => {
                    // Already in properties
                }
            }
        }
    }

    /// Resolve the sensitive fields stripped from a write into the opaque blob to persist.
    ///
    /// - No sensitive fields (`None`) → `Ok(None)`: nothing to store.
    /// - Sensitive fields present **and** an encryptor is configured → seal them and return the
    ///   blob.
    /// - Sensitive fields present but **no** encryptor (either the `encryption` feature is off, or
    ///   the store was built with [`new`](Self::new)) → [`Error::InvalidArgument`]. We refuse to
    ///   silently drop secret data: a resource with sensitive fields can only be written through a
    ///   store that can encrypt them.
    ///
    /// Serialization of the map to bytes happens here so the crypto layer only ever sees opaque
    /// bytes.
    async fn resolve_sensitive_blob(
        &self,
        id: &Uuid,
        label: L,
        sensitive: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<Option<bytes::Bytes>> {
        let Some(map) = sensitive else {
            return Ok(None);
        };

        #[cfg(feature = "encryption")]
        if let Some(encryptor) = self.encryptor.as_ref() {
            let plaintext = serde_json::to_vec(&serde_json::Value::Object(map))?;
            let blob = encryptor.seal(&secret_name(id), &plaintext).await?;
            return Ok(Some(bytes::Bytes::from(blob)));
        }

        let _ = (id, &map);
        Err(Error::invalid_argument(format!(
            "resource '{label}' has sensitive field(s) but the store has no encryptor configured \
             to seal them; build the store with `ManagedObjectStore::with_encryptor` (requires the \
             `encryption` feature)"
        )))
    }
}

/// The AAD name a sensitive blob is bound to: the object's stable [`Uuid`].
///
/// Binding to the id (not the [`ResourceName`]) means a [`rename`](ObjectStore::rename) needs no
/// re-sealing, and a blob cannot be opened against a different object.
#[cfg(feature = "encryption")]
fn secret_name(id: &Uuid) -> String {
    id.hyphenated().to_string()
}

// --- ObjectStoreReader impl ---

#[async_trait::async_trait]
impl<L: Label, S: ObjectStoreReader<L>> ObjectStoreReader<L> for ManagedObjectStore<L, S> {
    async fn get(&self, id: &Uuid) -> Result<Object<L>> {
        let mut object = self.inner.get(id).await?;
        self.inject_fields(&mut object);
        Ok(object)
    }

    async fn get_by_name(&self, label: L, name: &ResourceName) -> Result<Object<L>> {
        let mut object = self.inner.get_by_name(label, name).await?;
        self.inject_fields(&mut object);
        Ok(object)
    }

    async fn list(
        &self,
        label: L,
        namespace: Option<&ResourceName>,
        max_results: Option<usize>,
        page_token: Option<String>,
    ) -> Result<(Vec<Object<L>>, Option<String>)> {
        let (mut objects, token) = self
            .inner
            .list(label, namespace, max_results, page_token)
            .await?;
        for object in &mut objects {
            self.inject_fields(object);
        }
        Ok((objects, token))
    }

    async fn get_sensitive(&self, id: &Uuid) -> Result<Option<bytes::Bytes>> {
        self.inner.get_sensitive(id).await
    }
}

// --- ObjectStore impl ---

#[async_trait::async_trait]
impl<L: Label, S: ObjectStore<L>> ObjectStore<L> for ManagedObjectStore<L, S> {
    /// The managed store seals its own sensitive fields, so `sensitive` is normally `None`; a
    /// caller-supplied pre-sealed blob is used only when the resource has no sensitive fields to
    /// seal (the sealed blob otherwise takes precedence).
    async fn create(
        &self,
        label: L,
        name: &ResourceName,
        properties: Option<serde_json::Value>,
        id: Option<Uuid>,
        sensitive: Option<bytes::Bytes>,
    ) -> Result<Object<L>> {
        let (stripped, sensitive_fields) = self.strip_fields(label, properties);

        // Pre-allocate the id so the sealed blob can be bound to it before the row is written.
        // The object row and its sealed sensitive blob are written together in one atomic
        // `create`, so there is no orphan/rollback window.
        let id = id.unwrap_or_else(Uuid::new_v4);

        // Seal the sensitive fields (or error if there are any and no encryptor is configured).
        // A caller-supplied pre-sealed blob is used only when there are no sensitive fields.
        let blob = self
            .resolve_sensitive_blob(&id, label, sensitive_fields)
            .await?
            .or(sensitive);
        let mut object = self
            .inner
            .create(label, name, stripped, Some(id), blob)
            .await?;
        self.inject_fields(&mut object);
        Ok(object)
    }

    /// As with [`create`](Self::create), `sensitive` is normally `None`; the managed store seals
    /// any sensitive fields found in `properties` itself.
    async fn update(
        &self,
        id: &Uuid,
        properties: Option<serde_json::Value>,
        precondition: Precondition,
        sensitive: Option<bytes::Bytes>,
    ) -> Result<Object<L>> {
        // Look up the label to resolve field roles.
        let existing = self.inner.get(id).await?;
        let (stripped, sensitive_fields) = self.strip_fields(existing.label, properties);

        // A `None` blob leaves any existing sealed value untouched; a `Some` blob replaces it,
        // atomically with the properties update. Errors if sensitive fields are present but no
        // encryptor is configured.
        let blob = self
            .resolve_sensitive_blob(id, existing.label, sensitive_fields)
            .await?
            .or(sensitive);
        let mut object = self.inner.update(id, stripped, precondition, blob).await?;
        self.inject_fields(&mut object);
        Ok(object)
    }

    async fn rename(
        &self,
        id: &Uuid,
        new_name: &ResourceName,
        precondition: Precondition,
    ) -> Result<Object<L>> {
        // The sealed blob rides the object row and is bound to the stable id, so a rename needs
        // no re-sealing — just delegate.
        let mut object = self.inner.rename(id, new_name, precondition).await?;
        self.inject_fields(&mut object);
        Ok(object)
    }

    async fn delete(&self, id: &Uuid) -> Result<()> {
        // The sealed blob is stored on the object row, so deleting the object drops it too.
        self.inner.delete(id).await
    }
}

impl<L: Label, S: ObjectStore<L>> ManagedObjectStore<L, S> {
    /// Get an object with its sensitive fields decrypted and merged back into `properties`.
    ///
    /// Intended for internal use (e.g. credential vending) where the caller needs the full value.
    /// Without an encryptor, or when no blob is stored, this behaves like [`get`](Self::get).
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`] if no object with `id` exists, or a decryption error if the
    /// stored blob cannot be opened.
    pub async fn get_with_secrets(&self, id: &Uuid) -> Result<Object<L>> {
        let mut object = self.inner.get(id).await?;
        self.inject_fields(&mut object);

        #[cfg(feature = "encryption")]
        if let Some(encryptor) = self.encryptor.as_ref()
            && let Some(blob) = self.inner.get_sensitive(id).await?
        {
            let name = secret_name(id);
            let plaintext = encryptor.open(&name, &blob).await?;
            let sensitive: serde_json::Value = serde_json::from_slice(&plaintext)?;

            if let (Some(props), serde_json::Value::Object(secret_map)) =
                (object.properties.as_mut(), sensitive)
                && let Some(props_map) = props.as_object_mut()
            {
                for (key, value) in secret_map {
                    props_map.insert(key, value);
                }
            }

            // Lazy KEK rotation: if the blob was sealed under a retired KEK, re-wrap its data key
            // under the active KEK and write it back. Best-effort — a write failure must not fail
            // the read, and the value ciphertext is untouched so the result is identical.
            //
            // The writeback touches *only* the sealed blob via `set_sensitive`: we must not route
            // `object.properties` back through `update`, because by this point it has the decrypted
            // sensitive fields merged in — persisting that would leak plaintext into the (searchable,
            // unencrypted) properties column. `set_sensitive` also leaves the version/updated_at
            // untouched, so a read does not masquerade as a mutation.
            if let Ok(Some(rewrapped)) = encryptor.rewrap(&blob).await {
                let _ = self
                    .inner
                    .set_sensitive(id, bytes::Bytes::from(rewrapped))
                    .await;
            }
        }

        Ok(object)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InMemoryStore;
    use crate::registry::{ResourceFieldDescriptor, ResourceTypeDescriptor};
    use std::str::FromStr;

    use crate::Error;

    // --- A minimal Label implementation ---

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    enum TestLabel {
        Widget,
        Other,
    }

    impl std::fmt::Display for TestLabel {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.as_str())
        }
    }

    impl FromStr for TestLabel {
        type Err = Error;
        fn from_str(s: &str) -> Result<Self> {
            match s {
                "widget" => Ok(TestLabel::Widget),
                "other" => Ok(TestLabel::Other),
                _ => Err(Error::invalid_argument(format!("unknown label: {s}"))),
            }
        }
    }

    impl Label for TestLabel {
        fn as_str(&self) -> &str {
            match self {
                TestLabel::Widget => "widget",
                TestLabel::Other => "other",
            }
        }
    }

    // --- Registry fixture ---
    //
    // The "widget" resource has one field of each role so we can exercise stripping, injection,
    // redaction and secret routing in one place.

    static WIDGET_FIELDS: &[ResourceFieldDescriptor] = &[
        ResourceFieldDescriptor {
            name: "id",
            role: FieldRole::Identifier,
        },
        ResourceFieldDescriptor {
            name: "created_at",
            role: FieldRole::Managed,
        },
        ResourceFieldDescriptor {
            name: "updated_at",
            role: FieldRole::Managed,
        },
        ResourceFieldDescriptor {
            name: "color",
            role: FieldRole::Data,
        },
        ResourceFieldDescriptor {
            name: "api_key",
            role: FieldRole::Sensitive,
        },
    ];

    // "other" has no sensitive fields.
    static OTHER_FIELDS: &[ResourceFieldDescriptor] = &[ResourceFieldDescriptor {
        name: "value",
        role: FieldRole::Data,
    }];

    static DESCRIPTORS: &[ResourceTypeDescriptor<TestLabel>] = &[
        ResourceTypeDescriptor {
            label: TestLabel::Widget,
            fields: WIDGET_FIELDS,
            path_names: &["name"],
            parent_label: None,
        },
        ResourceTypeDescriptor {
            label: TestLabel::Other,
            fields: OTHER_FIELDS,
            path_names: &["name"],
            parent_label: None,
        },
    ];

    fn registry() -> ResourceRegistry<TestLabel> {
        ResourceRegistry::from_static(DESCRIPTORS)
    }

    fn rn(s: &str) -> ResourceName {
        ResourceName::from_naive_str_split(s)
    }

    fn props(json: serde_json::Value) -> Option<serde_json::Value> {
        Some(json)
    }

    #[cfg(feature = "encryption")]
    fn encryptor() -> crate::encryption::EnvelopeEncryptor {
        crate::encryption::EnvelopeEncryptor::local(
            crate::encryption::LocalKeyProvider::dev_insecure(),
        )
    }

    // --- ResourceRegistry tests ---

    #[test]
    fn registry_lookups() {
        let reg = registry();
        assert!(reg.get(TestLabel::Widget).is_some());
        assert!(reg.has_sensitive_fields(TestLabel::Widget));
        assert!(!reg.has_sensitive_fields(TestLabel::Other));
        assert_eq!(
            reg.sensitive_field_names(TestLabel::Widget),
            vec!["api_key"]
        );
        assert_eq!(reg.identifier_field_name(TestLabel::Widget), Some("id"));
        assert_eq!(reg.identifier_field_name(TestLabel::Other), None);
        let mut managed = reg.managed_field_names(TestLabel::Widget);
        managed.sort_unstable();
        assert_eq!(managed, vec!["created_at", "updated_at"]);
        assert_eq!(reg.parent_label(TestLabel::Widget), None);
        assert_eq!(reg.path_names(TestLabel::Widget), Some(&["name"][..]));
    }

    // --- Stripping + injection (no encryptor) ---

    #[tokio::test]
    async fn strips_managed_and_identifier_on_create_and_injects_on_read() {
        let store = ManagedObjectStore::new(InMemoryStore::<TestLabel>::new(), registry());

        // Caller supplies id/created_at (should be stripped) and color (kept).
        let created = store
            .create(
                TestLabel::Widget,
                &rn("w1"),
                props(serde_json::json!({
                    "id": "client-supplied-id",
                    "created_at": "client-supplied-time",
                    "color": "red",
                })),
                None,
                None,
            )
            .await
            .unwrap();

        let map = created.properties.as_ref().unwrap().as_object().unwrap();
        // Data field preserved.
        assert_eq!(map["color"], serde_json::json!("red"));
        // Identifier injected from the store's real id, not the client value.
        assert_eq!(map["id"], serde_json::json!(created.id.to_string()));
        // Managed created_at injected from the store, not the client value.
        assert_eq!(
            map["created_at"],
            serde_json::json!(created.created_at.to_rfc3339())
        );

        // Verify the underlying store never persisted the stripped fields.
        let raw = store.inner.get(&created.id).await.unwrap();
        let raw_map = raw.properties.as_ref().unwrap().as_object().unwrap();
        assert!(!raw_map.contains_key("id"));
        assert!(!raw_map.contains_key("created_at"));
        assert_eq!(raw_map["color"], serde_json::json!("red"));
    }

    #[tokio::test]
    async fn writing_sensitive_field_without_encryptor_errors() {
        let store = ManagedObjectStore::new(InMemoryStore::<TestLabel>::new(), registry());

        // A resource with a sensitive field cannot be written without an encryptor: we refuse to
        // silently drop the secret.
        let err = store
            .create(
                TestLabel::Widget,
                &rn("w1"),
                props(serde_json::json!({ "color": "blue", "api_key": "supersecret" })),
                None,
                None,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgument(_)),
            "expected InvalidArgument, got {err:?}"
        );

        // Nothing was persisted.
        assert!(
            store
                .get_by_name(TestLabel::Widget, &rn("w1"))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn write_without_sensitive_fields_succeeds_without_encryptor() {
        let store = ManagedObjectStore::new(InMemoryStore::<TestLabel>::new(), registry());

        // A resource with no sensitive field in the payload writes fine without an encryptor.
        let created = store
            .create(
                TestLabel::Widget,
                &rn("w1"),
                props(serde_json::json!({ "color": "blue" })),
                None,
                None,
            )
            .await
            .unwrap();
        let map = created.properties.as_ref().unwrap().as_object().unwrap();
        assert_eq!(map["color"], serde_json::json!("blue"));
        assert!(!map.contains_key("api_key"));
        // "other" has no sensitive fields at all, so it also writes fine.
        store
            .create(
                TestLabel::Other,
                &rn("o1"),
                props(serde_json::json!({ "value": "v" })),
                None,
                None,
            )
            .await
            .unwrap();
    }

    // --- Sealing + redaction (with encryptor) ---

    #[cfg(feature = "encryption")]
    #[tokio::test]
    async fn sensitive_value_sealed_and_redacted_on_get() {
        let store = ManagedObjectStore::with_encryptor(
            InMemoryStore::<TestLabel>::new(),
            encryptor(),
            registry(),
        );

        let created = store
            .create(
                TestLabel::Widget,
                &rn("w1"),
                props(serde_json::json!({ "color": "green", "api_key": "topsecret" })),
                None,
                None,
            )
            .await
            .unwrap();

        // The api_key must NOT be in the object properties (redacted).
        let map = created.properties.as_ref().unwrap().as_object().unwrap();
        assert!(!map.contains_key("api_key"));
        assert_eq!(map["color"], serde_json::json!("green"));

        // A sealed blob is stored on the row, and it is ciphertext (no plaintext substring).
        let blob = store
            .inner
            .get_sensitive(&created.id)
            .await
            .unwrap()
            .expect("blob stored");
        assert!(!blob.windows(b"topsecret".len()).any(|w| w == b"topsecret"));

        // A plain get redacts the secret.
        let got = store.get(&created.id).await.unwrap();
        assert!(
            !got.properties
                .as_ref()
                .unwrap()
                .as_object()
                .unwrap()
                .contains_key("api_key")
        );

        // get_with_secrets joins the full value back in.
        let full = store.get_with_secrets(&created.id).await.unwrap();
        let full_map = full.properties.as_ref().unwrap().as_object().unwrap();
        assert_eq!(full_map["api_key"], serde_json::json!("topsecret"));
        assert_eq!(full_map["color"], serde_json::json!("green"));
    }

    #[cfg(feature = "encryption")]
    #[tokio::test]
    async fn update_seals_new_sensitive_value() {
        let store = ManagedObjectStore::with_encryptor(
            InMemoryStore::<TestLabel>::new(),
            encryptor(),
            registry(),
        );

        // Create without a secret value, so no blob exists yet.
        let created = store
            .create(
                TestLabel::Widget,
                &rn("w1"),
                props(serde_json::json!({ "color": "green" })),
                None,
                None,
            )
            .await
            .unwrap();
        assert!(
            store
                .inner
                .get_sensitive(&created.id)
                .await
                .unwrap()
                .is_none()
        );

        // Update with a sensitive value -> a blob is sealed and stored.
        store
            .update(
                &created.id,
                props(serde_json::json!({ "color": "green", "api_key": "k1" })),
                Precondition::Any,
                None,
            )
            .await
            .unwrap();

        let full = store.get_with_secrets(&created.id).await.unwrap();
        assert_eq!(
            full.properties.as_ref().unwrap().as_object().unwrap()["api_key"],
            serde_json::json!("k1")
        );
    }

    #[cfg(feature = "encryption")]
    #[tokio::test]
    async fn update_overwrites_existing_secret() {
        let store = ManagedObjectStore::with_encryptor(
            InMemoryStore::<TestLabel>::new(),
            encryptor(),
            registry(),
        );

        let created = store
            .create(
                TestLabel::Widget,
                &rn("w1"),
                props(serde_json::json!({ "api_key": "old" })),
                None,
                None,
            )
            .await
            .unwrap();

        store
            .update(
                &created.id,
                props(serde_json::json!({ "api_key": "new" })),
                Precondition::Any,
                None,
            )
            .await
            .unwrap();

        let full = store.get_with_secrets(&created.id).await.unwrap();
        assert_eq!(
            full.properties.as_ref().unwrap().as_object().unwrap()["api_key"],
            serde_json::json!("new")
        );
    }

    #[cfg(feature = "encryption")]
    #[tokio::test]
    async fn update_without_sensitive_fields_preserves_blob() {
        let store = ManagedObjectStore::with_encryptor(
            InMemoryStore::<TestLabel>::new(),
            encryptor(),
            registry(),
        );

        let created = store
            .create(
                TestLabel::Widget,
                &rn("w1"),
                props(serde_json::json!({ "color": "green", "api_key": "keep-me" })),
                None,
                None,
            )
            .await
            .unwrap();

        // Update only the data field; no sensitive field in the payload.
        store
            .update(
                &created.id,
                props(serde_json::json!({ "color": "blue" })),
                Precondition::Any,
                None,
            )
            .await
            .unwrap();

        // The previously sealed value must still be reconstitutable.
        let full = store.get_with_secrets(&created.id).await.unwrap();
        let full_map = full.properties.as_ref().unwrap().as_object().unwrap();
        assert_eq!(full_map["api_key"], serde_json::json!("keep-me"));
        assert_eq!(full_map["color"], serde_json::json!("blue"));
    }

    #[cfg(feature = "encryption")]
    #[tokio::test]
    async fn delete_removes_sealed_blob() {
        let store = ManagedObjectStore::with_encryptor(
            InMemoryStore::<TestLabel>::new(),
            encryptor(),
            registry(),
        );

        let created = store
            .create(
                TestLabel::Widget,
                &rn("w1"),
                props(serde_json::json!({ "api_key": "s" })),
                None,
                None,
            )
            .await
            .unwrap();
        assert!(
            store
                .inner
                .get_sensitive(&created.id)
                .await
                .unwrap()
                .is_some()
        );

        store.delete(&created.id).await.unwrap();
        assert!(matches!(
            store.get(&created.id).await.unwrap_err(),
            Error::NotFound
        ));
        // The blob is gone with the row.
        assert!(
            store
                .inner
                .get_sensitive(&created.id)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[cfg(feature = "encryption")]
    #[tokio::test]
    async fn rotation_rewraps_on_read() {
        use crate::encryption::{EnvelopeEncryptor, LocalKeyProvider};

        let k1 = vec![1u8; 32];
        let k2 = vec![2u8; 32];

        // Seal under v1.
        let store_v1 = ManagedObjectStore::with_encryptor(
            InMemoryStore::<TestLabel>::new(),
            EnvelopeEncryptor::local(LocalKeyProvider::single("v1", k1.clone()).unwrap()),
            registry(),
        );
        let created = store_v1
            .create(
                TestLabel::Widget,
                &rn("w1"),
                props(serde_json::json!({ "api_key": "rotate-me" })),
                None,
                None,
            )
            .await
            .unwrap();

        // Move the same inner store under a v2-active/v1-retired encryptor.
        let inner = store_v1.inner;
        let store_v2 = ManagedObjectStore::with_encryptor(
            inner,
            EnvelopeEncryptor::local(
                LocalKeyProvider::new("v2", [("v1".into(), k1), ("v2".into(), k2.clone())])
                    .unwrap(),
            ),
            registry(),
        );

        // Reading with secrets still works and rewrites the row under v2.
        let full = store_v2.get_with_secrets(&created.id).await.unwrap();
        assert_eq!(
            full.properties.as_ref().unwrap().as_object().unwrap()["api_key"],
            serde_json::json!("rotate-me")
        );

        // The lazy-rotation writeback must NOT leak the decrypted secret into the stored
        // properties, and must NOT bump the version (a read is not a mutation).
        let raw = store_v2.inner.get(&created.id).await.unwrap();
        assert!(
            raw.properties
                .as_ref()
                .and_then(|p| p.as_object())
                .is_none_or(|m| !m.contains_key("api_key")),
            "rotation writeback leaked plaintext into stored properties"
        );
        assert_eq!(
            raw.version, created.version,
            "rotation writeback must not bump the object version"
        );
        // A plain get stays redacted after rotation.
        let redacted = store_v2.get(&created.id).await.unwrap();
        assert!(
            !redacted
                .properties
                .as_ref()
                .unwrap()
                .as_object()
                .unwrap()
                .contains_key("api_key")
        );

        // A v2-only encryptor can now open the rewritten blob.
        let store_v2_only = ManagedObjectStore::with_encryptor(
            store_v2.inner,
            EnvelopeEncryptor::local(LocalKeyProvider::single("v2", k2).unwrap()),
            registry(),
        );
        let full = store_v2_only.get_with_secrets(&created.id).await.unwrap();
        assert_eq!(
            full.properties.as_ref().unwrap().as_object().unwrap()["api_key"],
            serde_json::json!("rotate-me")
        );
    }
}
