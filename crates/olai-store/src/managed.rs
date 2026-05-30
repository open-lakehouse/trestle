//! Registry-aware object store decorator that enforces field roles.
//!
//! [`ManagedObjectStore`] wraps an [`ObjectStore`] and optionally a [`SecretManager`]
//! to automatically:
//!
//! - Strip [`FieldRole::Identifier`] and [`FieldRole::Managed`] fields on create/update
//!   (the store is the source of truth for these)
//! - Route [`FieldRole::Sensitive`] fields to the [`SecretManager`]
//! - Inject Identifier and Managed fields back into properties on read
//! - Redact Sensitive fields on read (unless `get_with_secrets` is used)

use std::marker::PhantomData;
use std::sync::Arc;

use bytes::Bytes;
use uuid::Uuid;

use crate::label::Label;
use crate::name::ResourceName;
use crate::object::Object;
use crate::registry::{FieldRole, ResourceRegistry};
use crate::secrets::SecretManager;
use crate::store::{ObjectStore, ObjectStoreReader};
use crate::{Error, Result};

/// A registry-aware object store that enforces field roles.
///
/// Wraps an inner [`ObjectStore`] and uses a [`ResourceRegistry`] to determine
/// how each field should be handled during CRUD operations.
///
/// When a [`SecretManager`] is provided, sensitive fields (marked with
/// `debug_redact = true` in proto definitions) are automatically separated
/// into encrypted secret storage.
pub struct ManagedObjectStore<L: Label, S, M = NoSecrets> {
    inner: S,
    secrets: M,
    registry: Arc<ResourceRegistry<L>>,
    _label: PhantomData<L>,
}

/// Placeholder type for when no [`SecretManager`] is configured.
pub struct NoSecrets;

impl<L: Label, S: ObjectStore<L>> ManagedObjectStore<L, S, NoSecrets> {
    /// Create a managed store without secret management.
    ///
    /// Sensitive fields will be stripped from properties but not stored anywhere.
    pub fn new(inner: S, registry: ResourceRegistry<L>) -> Self {
        Self {
            inner,
            secrets: NoSecrets,
            registry: Arc::new(registry),
            _label: PhantomData,
        }
    }
}

impl<L: Label, S: ObjectStore<L>, M: SecretManager> ManagedObjectStore<L, S, M> {
    /// Create a managed store with secret management.
    pub fn with_secrets(inner: S, secrets: M, registry: ResourceRegistry<L>) -> Self {
        Self {
            inner,
            secrets,
            registry: Arc::new(registry),
            _label: PhantomData,
        }
    }
}

impl<L: Label, S, M> ManagedObjectStore<L, S, M> {
    /// Strip fields that should not be stored in properties on create/update.
    ///
    /// Returns (stripped_properties, sensitive_fields_map).
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
                    // Extract — will be routed to secret store
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
                    // Redact: ensure sensitive fields are null in the response
                    map.remove(field.name);
                }
                FieldRole::Data => {
                    // Already in properties
                }
            }
        }
    }
}

// --- ObjectStoreReader impl ---

#[async_trait::async_trait]
impl<L: Label, S: ObjectStoreReader<L>, M: Send + Sync + 'static> ObjectStoreReader<L>
    for ManagedObjectStore<L, S, M>
{
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
}

// --- ObjectStore impl (without secrets) ---

#[async_trait::async_trait]
impl<L: Label, S: ObjectStore<L>> ObjectStore<L> for ManagedObjectStore<L, S, NoSecrets> {
    async fn create(
        &self,
        label: L,
        name: &ResourceName,
        properties: Option<serde_json::Value>,
    ) -> Result<Object<L>> {
        let (stripped, _sensitive) = self.strip_fields(label, properties);
        let mut object = self.inner.create(label, name, stripped).await?;
        self.inject_fields(&mut object);
        Ok(object)
    }

    async fn update(&self, id: &Uuid, properties: Option<serde_json::Value>) -> Result<Object<L>> {
        // We need the label to look up the descriptor. Fetch the object first.
        let existing = self.inner.get(id).await?;
        let (stripped, _sensitive) = self.strip_fields(existing.label, properties);
        let mut object = self.inner.update(id, stripped).await?;
        self.inject_fields(&mut object);
        Ok(object)
    }

    async fn delete(&self, id: &Uuid) -> Result<()> {
        self.inner.delete(id).await
    }
}

// --- ObjectStore impl (with secrets) ---

#[async_trait::async_trait]
impl<L: Label, S: ObjectStore<L>, M: SecretManager> ObjectStore<L> for ManagedObjectStore<L, S, M> {
    async fn create(
        &self,
        label: L,
        name: &ResourceName,
        properties: Option<serde_json::Value>,
    ) -> Result<Object<L>> {
        let (stripped, sensitive) = self.strip_fields(label, properties);

        // Store sensitive fields in secret manager
        if let Some(sensitive_map) = sensitive {
            let secret_bytes = Bytes::from(serde_json::to_vec(&serde_json::Value::Object(
                sensitive_map,
            ))?);
            self.secrets
                .create_secret(&name.to_string(), secret_bytes)
                .await?;
        }

        let mut object = self.inner.create(label, name, stripped).await?;
        self.inject_fields(&mut object);
        Ok(object)
    }

    /// Update an object, routing sensitive fields to the [`SecretManager`].
    ///
    /// Sensitive fields are written to the secret store *before* the inner object
    /// store is updated. If the inner update then fails, we issue a best-effort
    /// compensating delete of the just-written secret so it does not linger as an
    /// orphan referencing a row that was never updated.
    ///
    /// # Residual risk
    ///
    /// This is best-effort, not a transaction. A window remains where the secret
    /// write has succeeded but the object write has not yet committed:
    ///
    /// - If the process crashes between the two writes, the compensating delete
    ///   never runs and the secret is orphaned (or, for a previously-existing
    ///   secret, left holding the new value while the object keeps the old data).
    /// - The compensating delete can itself fail (it is logged and ignored), again
    ///   leaving an orphan.
    /// - When the secret already existed, the compensating delete *removes* it
    ///   entirely rather than restoring the prior value — we do not snapshot and
    ///   roll back the old secret. A subsequent read will see the secret as missing.
    ///
    /// Callers that need stronger guarantees should layer a reconciliation/GC pass
    /// over the secret store, keyed by resource name, to reap orphans.
    async fn update(&self, id: &Uuid, properties: Option<serde_json::Value>) -> Result<Object<L>> {
        let existing = self.inner.get(id).await?;
        let (stripped, sensitive) = self.strip_fields(existing.label, properties);

        // Track whether we wrote a secret, so we can compensate if the inner
        // update below fails.
        let mut wrote_secret_name: Option<String> = None;

        // Update sensitive fields in secret manager
        if let Some(sensitive_map) = sensitive {
            let secret_bytes = Bytes::from(serde_json::to_vec(&serde_json::Value::Object(
                sensitive_map,
            ))?);
            let secret_name = existing.name.to_string();
            // Try update; if the secret doesn't exist yet, create it
            match self
                .secrets
                .update_secret(&secret_name, secret_bytes.clone())
                .await
            {
                Ok(_) => {}
                Err(Error::NotFound) => {
                    self.secrets
                        .create_secret(&secret_name, secret_bytes)
                        .await?;
                }
                Err(e) => return Err(e),
            }
            wrote_secret_name = Some(secret_name);
        }

        let object = match self.inner.update(id, stripped).await {
            Ok(object) => object,
            Err(e) => {
                // The inner store write failed after we already wrote the secret.
                // Best-effort: undo the secret write so it does not orphan. Errors
                // from the compensating delete are swallowed (logged) — we surface
                // the original update error to the caller regardless.
                if let Some(secret_name) = wrote_secret_name {
                    if let Err(cleanup_err) = self.secrets.delete_secret(&secret_name).await {
                        tracing::warn!(
                            secret_name = %secret_name,
                            error = %cleanup_err,
                            "failed to compensate (delete) orphaned secret after inner store \
                             update failed; secret may be orphaned"
                        );
                    }
                }
                return Err(e);
            }
        };

        let mut object = object;
        self.inject_fields(&mut object);
        Ok(object)
    }

    async fn delete(&self, id: &Uuid) -> Result<()> {
        // Delete secret first (best-effort — may not exist)
        let object = self.inner.get(id).await?;
        if self.registry.has_sensitive_fields(object.label) {
            let secret_name = object.name.to_string();
            match self.secrets.delete_secret(&secret_name).await {
                Ok(()) | Err(Error::NotFound) => {}
                Err(e) => return Err(e),
            }
        }
        self.inner.delete(id).await
    }
}

impl<L: Label, S: ObjectStore<L>, M: SecretManager> ManagedObjectStore<L, S, M> {
    /// Get an object with its sensitive fields populated from the secret store.
    ///
    /// This is intended for internal use (e.g., credential vending) where the
    /// caller needs access to the full credential data.
    pub async fn get_with_secrets(&self, id: &Uuid) -> Result<Object<L>> {
        let mut object = self.inner.get(id).await?;
        self.inject_fields(&mut object);

        // Join sensitive fields from secret store
        if self.registry.has_sensitive_fields(object.label) {
            let secret_name = object.name.to_string();
            match self.secrets.get_secret(&secret_name).await {
                Ok((_version, secret_bytes)) => {
                    let sensitive: serde_json::Value = serde_json::from_slice(&secret_bytes)?;
                    if let (Some(props), serde_json::Value::Object(secret_map)) =
                        (object.properties.as_mut(), sensitive)
                    {
                        if let Some(props_map) = props.as_object_mut() {
                            for (key, value) in secret_map {
                                props_map.insert(key, value);
                            }
                        }
                    }
                }
                Err(Error::NotFound) => {
                    // No secrets stored — that's fine
                }
                Err(e) => return Err(e),
            }
        }

        Ok(object)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::Association;
    use crate::registry::{ResourceFieldDescriptor, ResourceTypeDescriptor};
    use crate::store::{AssociationStore, AssociationStoreReader};
    use std::collections::HashMap;
    use std::str::FromStr;
    use std::sync::Mutex;

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
    // The "widget" resource has one field of each role so we can exercise
    // stripping, injection, redaction and secret routing in one place.

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

    // --- In-memory ObjectStore double ---

    #[derive(Default)]
    struct MemObjectStore {
        objects: Mutex<HashMap<Uuid, Object<TestLabel>>>,
        /// When set, the next `update` call fails with this error (then clears).
        fail_next_update: Mutex<Option<Error>>,
    }

    impl MemObjectStore {
        fn fail_next_update_with(&self, err: Error) {
            *self.fail_next_update.lock().unwrap() = Some(err);
        }
    }

    #[async_trait::async_trait]
    impl ObjectStoreReader<TestLabel> for MemObjectStore {
        async fn get(&self, id: &Uuid) -> Result<Object<TestLabel>> {
            self.objects
                .lock()
                .unwrap()
                .get(id)
                .cloned()
                .ok_or(Error::NotFound)
        }

        async fn get_by_name(
            &self,
            label: TestLabel,
            name: &ResourceName,
        ) -> Result<Object<TestLabel>> {
            self.objects
                .lock()
                .unwrap()
                .values()
                .find(|o| o.label == label && &o.name == name)
                .cloned()
                .ok_or(Error::NotFound)
        }

        async fn list(
            &self,
            label: TestLabel,
            _namespace: Option<&ResourceName>,
            _max_results: Option<usize>,
            _page_token: Option<String>,
        ) -> Result<(Vec<Object<TestLabel>>, Option<String>)> {
            let objects = self
                .objects
                .lock()
                .unwrap()
                .values()
                .filter(|o| o.label == label)
                .cloned()
                .collect();
            Ok((objects, None))
        }
    }

    #[async_trait::async_trait]
    impl ObjectStore<TestLabel> for MemObjectStore {
        async fn create(
            &self,
            label: TestLabel,
            name: &ResourceName,
            properties: Option<serde_json::Value>,
        ) -> Result<Object<TestLabel>> {
            let object = Object {
                id: Uuid::new_v4(),
                label,
                name: name.clone(),
                properties,
                created_at: chrono::Utc::now(),
                updated_at: None,
            };
            self.objects
                .lock()
                .unwrap()
                .insert(object.id, object.clone());
            Ok(object)
        }

        async fn update(
            &self,
            id: &Uuid,
            properties: Option<serde_json::Value>,
        ) -> Result<Object<TestLabel>> {
            if let Some(err) = self.fail_next_update.lock().unwrap().take() {
                return Err(err);
            }
            let mut guard = self.objects.lock().unwrap();
            let object = guard.get_mut(id).ok_or(Error::NotFound)?;
            object.properties = properties;
            object.updated_at = Some(chrono::Utc::now());
            Ok(object.clone())
        }

        async fn delete(&self, id: &Uuid) -> Result<()> {
            self.objects
                .lock()
                .unwrap()
                .remove(id)
                .map(|_| ())
                .ok_or(Error::NotFound)
        }
    }

    // --- In-memory AssociationStore double ---

    #[derive(Default)]
    struct MemAssociationStore {
        edges: Mutex<Vec<Association<TestLabel>>>,
    }

    #[async_trait::async_trait]
    impl AssociationStoreReader<TestLabel> for MemAssociationStore {
        async fn list(
            &self,
            from_id: Uuid,
            label: &str,
            target_label: Option<TestLabel>,
            _max_results: Option<usize>,
            _page_token: Option<String>,
        ) -> Result<(Vec<Association<TestLabel>>, Option<String>)> {
            let edges = self
                .edges
                .lock()
                .unwrap()
                .iter()
                .filter(|e| {
                    e.from_id == from_id
                        && e.label == label
                        && target_label.is_none_or(|tl| e.to_label == tl)
                })
                .cloned()
                .collect();
            Ok((edges, None))
        }
    }

    #[async_trait::async_trait]
    impl AssociationStore<TestLabel> for MemAssociationStore {
        async fn add(
            &self,
            from_id: Uuid,
            to_id: Uuid,
            label: &str,
            properties: Option<serde_json::Value>,
        ) -> Result<()> {
            self.edges.lock().unwrap().push(Association {
                id: Uuid::new_v4(),
                from_id,
                label: label.to_string(),
                to_id,
                to_label: TestLabel::Widget,
                properties,
                created_at: chrono::Utc::now(),
                updated_at: None,
            });
            Ok(())
        }

        async fn remove(&self, from_id: Uuid, to_id: Uuid, label: &str) -> Result<()> {
            self.edges
                .lock()
                .unwrap()
                .retain(|e| !(e.from_id == from_id && e.to_id == to_id && e.label == label));
            Ok(())
        }
    }

    // --- In-memory SecretManager double ---

    #[derive(Default)]
    struct MemSecretManager {
        secrets: Mutex<HashMap<String, (Uuid, Bytes)>>,
        /// When set, the next `delete_secret` call fails with this error.
        fail_next_delete: Mutex<Option<Error>>,
    }

    impl MemSecretManager {
        fn contains(&self, name: &str) -> bool {
            self.secrets.lock().unwrap().contains_key(name)
        }

        fn fail_next_delete_with(&self, err: Error) {
            *self.fail_next_delete.lock().unwrap() = Some(err);
        }
    }

    #[async_trait::async_trait]
    impl SecretManager for MemSecretManager {
        async fn get_secret(&self, secret_name: &str) -> Result<(Uuid, Bytes)> {
            self.secrets
                .lock()
                .unwrap()
                .get(secret_name)
                .cloned()
                .ok_or(Error::NotFound)
        }

        async fn get_secret_version(&self, secret_name: &str, version: Uuid) -> Result<Bytes> {
            let guard = self.secrets.lock().unwrap();
            match guard.get(secret_name) {
                Some((v, bytes)) if *v == version => Ok(bytes.clone()),
                Some(_) => Err(Error::NotFound),
                None => Err(Error::NotFound),
            }
        }

        async fn create_secret(&self, secret_name: &str, secret_value: Bytes) -> Result<Uuid> {
            let mut guard = self.secrets.lock().unwrap();
            if guard.contains_key(secret_name) {
                return Err(Error::AlreadyExists);
            }
            let version = Uuid::new_v4();
            guard.insert(secret_name.to_string(), (version, secret_value));
            Ok(version)
        }

        async fn update_secret(&self, secret_name: &str, secret_value: Bytes) -> Result<Uuid> {
            let mut guard = self.secrets.lock().unwrap();
            if !guard.contains_key(secret_name) {
                return Err(Error::NotFound);
            }
            let version = Uuid::new_v4();
            guard.insert(secret_name.to_string(), (version, secret_value));
            Ok(version)
        }

        async fn delete_secret(&self, secret_name: &str) -> Result<()> {
            if let Some(err) = self.fail_next_delete.lock().unwrap().take() {
                return Err(err);
            }
            self.secrets
                .lock()
                .unwrap()
                .remove(secret_name)
                .map(|_| ())
                .ok_or(Error::NotFound)
        }
    }

    fn props(json: serde_json::Value) -> Option<serde_json::Value> {
        Some(json)
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

    // --- NoSecrets variant: stripping + injection ---

    #[tokio::test]
    async fn no_secrets_strips_managed_and_identifier_on_create_and_injects_on_read() {
        let store = ManagedObjectStore::new(MemObjectStore::default(), registry());

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
    async fn no_secrets_strips_sensitive_but_does_not_store_it() {
        let store = ManagedObjectStore::new(MemObjectStore::default(), registry());

        let created = store
            .create(
                TestLabel::Widget,
                &rn("w1"),
                props(serde_json::json!({ "color": "blue", "api_key": "supersecret" })),
            )
            .await
            .unwrap();

        // Sensitive field stripped from the returned/redacted object.
        let map = created.properties.as_ref().unwrap().as_object().unwrap();
        assert!(!map.contains_key("api_key"));

        // And not persisted in the inner store (no secret manager to route to).
        let raw = store.inner.get(&created.id).await.unwrap();
        let raw_map = raw.properties.as_ref().unwrap().as_object().unwrap();
        assert!(!raw_map.contains_key("api_key"));
    }

    // --- Secret routing + redaction ---

    #[tokio::test]
    async fn sensitive_value_routed_to_secret_manager_and_redacted_on_get() {
        let store = ManagedObjectStore::with_secrets(
            MemObjectStore::default(),
            MemSecretManager::default(),
            registry(),
        );

        let created = store
            .create(
                TestLabel::Widget,
                &rn("w1"),
                props(serde_json::json!({ "color": "green", "api_key": "topsecret" })),
            )
            .await
            .unwrap();

        // The api_key must NOT be in the object properties (redacted).
        let map = created.properties.as_ref().unwrap().as_object().unwrap();
        assert!(!map.contains_key("api_key"));
        assert_eq!(map["color"], serde_json::json!("green"));

        // The api_key MUST be in the secret manager, keyed by resource name.
        assert!(store.secrets.contains("w1"));

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

    // --- update -> NotFound -> create fallback ---

    #[tokio::test]
    async fn update_creates_secret_when_missing() {
        let store = ManagedObjectStore::with_secrets(
            MemObjectStore::default(),
            MemSecretManager::default(),
            registry(),
        );

        // Create without a secret value, so no secret exists yet.
        let created = store
            .create(
                TestLabel::Widget,
                &rn("w1"),
                props(serde_json::json!({ "color": "green" })),
            )
            .await
            .unwrap();
        assert!(!store.secrets.contains("w1"));

        // Update with a sensitive value -> update_secret returns NotFound -> create.
        store
            .update(
                &created.id,
                props(serde_json::json!({ "color": "green", "api_key": "k1" })),
            )
            .await
            .unwrap();

        assert!(store.secrets.contains("w1"));
        let full = store.get_with_secrets(&created.id).await.unwrap();
        assert_eq!(
            full.properties.as_ref().unwrap().as_object().unwrap()["api_key"],
            serde_json::json!("k1")
        );
    }

    #[tokio::test]
    async fn update_overwrites_existing_secret() {
        let store = ManagedObjectStore::with_secrets(
            MemObjectStore::default(),
            MemSecretManager::default(),
            registry(),
        );

        let created = store
            .create(
                TestLabel::Widget,
                &rn("w1"),
                props(serde_json::json!({ "api_key": "old" })),
            )
            .await
            .unwrap();
        assert!(store.secrets.contains("w1"));

        store
            .update(&created.id, props(serde_json::json!({ "api_key": "new" })))
            .await
            .unwrap();

        let full = store.get_with_secrets(&created.id).await.unwrap();
        assert_eq!(
            full.properties.as_ref().unwrap().as_object().unwrap()["api_key"],
            serde_json::json!("new")
        );
    }

    // --- Compensating delete on inner-store failure (Task 1.6) ---

    #[tokio::test]
    async fn update_compensates_secret_when_inner_update_fails() {
        let store = ManagedObjectStore::with_secrets(
            MemObjectStore::default(),
            MemSecretManager::default(),
            registry(),
        );

        // Start with no secret stored.
        let created = store
            .create(
                TestLabel::Widget,
                &rn("w1"),
                props(serde_json::json!({ "color": "green" })),
            )
            .await
            .unwrap();
        assert!(!store.secrets.contains("w1"));

        // Arrange for the inner update to fail.
        store.inner.fail_next_update_with(Error::generic("boom"));

        let err = store
            .update(
                &created.id,
                props(serde_json::json!({ "color": "green", "api_key": "leaked?" })),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Generic(_)));

        // The secret that was created during the failed update must be compensated away.
        assert!(
            !store.secrets.contains("w1"),
            "secret should have been compensated (deleted) after inner update failed"
        );
    }

    #[tokio::test]
    async fn update_surfaces_original_error_even_if_compensating_delete_fails() {
        let store = ManagedObjectStore::with_secrets(
            MemObjectStore::default(),
            MemSecretManager::default(),
            registry(),
        );

        let created = store
            .create(
                TestLabel::Widget,
                &rn("w1"),
                props(serde_json::json!({ "color": "green" })),
            )
            .await
            .unwrap();

        store
            .inner
            .fail_next_update_with(Error::generic("inner failure"));
        // Make the compensating delete itself fail; the error should be swallowed/logged.
        store
            .secrets
            .fail_next_delete_with(Error::generic("delete failure"));

        let err = store
            .update(&created.id, props(serde_json::json!({ "api_key": "x" })))
            .await
            .unwrap_err();

        // The ORIGINAL inner error is surfaced, not the cleanup error.
        match err {
            Error::Generic(msg) => assert_eq!(msg, "inner failure"),
            other => panic!("expected original inner failure, got {other:?}"),
        }
    }

    // --- delete removes the secret ---

    #[tokio::test]
    async fn delete_removes_associated_secret() {
        let store = ManagedObjectStore::with_secrets(
            MemObjectStore::default(),
            MemSecretManager::default(),
            registry(),
        );

        let created = store
            .create(
                TestLabel::Widget,
                &rn("w1"),
                props(serde_json::json!({ "api_key": "s" })),
            )
            .await
            .unwrap();
        assert!(store.secrets.contains("w1"));

        store.delete(&created.id).await.unwrap();
        assert!(!store.secrets.contains("w1"));
        assert!(matches!(
            store.get(&created.id).await.unwrap_err(),
            Error::NotFound
        ));
    }

    // --- AssociationStore double sanity (exercises that trait too) ---

    #[tokio::test]
    async fn association_store_add_list_remove() {
        let store = MemAssociationStore::default();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();

        store.add(a, b, "parent_of", None).await.unwrap();
        let (edges, token) = store.list(a, "parent_of", None, None, None).await.unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to_id, b);
        assert!(token.is_none());

        store.remove(a, b, "parent_of").await.unwrap();
        let (edges, _) = store.list(a, "parent_of", None, None, None).await.unwrap();
        assert!(edges.is_empty());
    }
}
