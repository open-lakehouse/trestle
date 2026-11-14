//! Grants: which subject holds which privilege on which object.
//!
//! A [`Grant`] is a `(subject, privilege, object)` tuple. Subjects are users, service
//! accounts and groups ([`Subject`]); groups contain users, service accounts and other
//! groups. Objects are Cedar entities ([`ObjectRef`], e.g. `Ns::Folder::"inbox"`), and
//! privileges are names the host's Cedar policies give meaning to (`READ`, `OWNER`, ...).
//! A host keeps its own privilege vocabulary; this layer only checks the shape of a name
//! ([`validate_privilege`]).
//!
//! A [`GrantStore`] reads and writes them. [`MemoryGrantStore`] keeps them in memory, for
//! tests and single-process deployments; `PgGrantStore` (feature `postgres`) keeps them
//! in the tables of the `authz` migrations. `GrantsEntityLoader` (feature `cedar`) turns
//! them into the entities a `CedarAuthorizer` decides on.
//!
//! Objects whose ids are hierarchical names (`a`, `a.b`, `a.b.c`) can be deleted or
//! renamed together with their descendants: those of the given types whose id starts
//! with the object's id and a separator (`.` unless configured otherwise).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Mutex;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::BoxError;
use crate::wire::EntityRef;

#[cfg(feature = "cedar")]
pub(crate) mod loader;
#[cfg(feature = "postgres")]
pub(crate) mod postgres;

/// The object a grant is on: a Cedar entity uid.
pub type ObjectRef = EntityRef;

/// The separator of hierarchical object ids unless configured otherwise.
pub const DEFAULT_SEPARATOR: &str = ".";

/// What kind of subject holds a grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubjectKind {
    /// A user of the identity provider, by its subject (`sub`).
    User,
    /// A service account, by its id.
    ServiceAccount,
    /// A group, by its id.
    Group,
}

impl SubjectKind {
    /// The kind as stored: `user`, `service_account` or `group`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::ServiceAccount => "service_account",
            Self::Group => "group",
        }
    }

    /// The kind stored as `kind`.
    pub fn parse(kind: &str) -> Option<Self> {
        match kind {
            "user" => Some(Self::User),
            "service_account" => Some(Self::ServiceAccount),
            "group" => Some(Self::Group),
            _ => None,
        }
    }
}

/// Who holds a grant.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Subject {
    /// The kind of subject.
    pub kind: SubjectKind,
    /// Its id: a user's subject, or a service account's or group's id.
    pub id: String,
}

impl Subject {
    /// The subject `id` of `kind`.
    pub fn new(kind: SubjectKind, id: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
        }
    }

    /// The user with subject `sub`.
    pub fn user(sub: impl Into<String>) -> Self {
        Self::new(SubjectKind::User, sub)
    }

    /// The service account `id`.
    pub fn service_account(id: impl Into<String>) -> Self {
        Self::new(SubjectKind::ServiceAccount, id)
    }

    /// The group `id`.
    pub fn group(id: impl Into<String>) -> Self {
        Self::new(SubjectKind::Group, id)
    }
}

/// `subject` holds `privilege` on `object`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Grant {
    /// Who holds it.
    pub subject: Subject,
    /// The privilege, e.g. `READ`.
    pub privilege: String,
    /// What it is on.
    pub object: ObjectRef,
}

/// Privileges to add and remove for one subject on one object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantChange {
    /// Whose privileges change.
    pub subject: Subject,
    /// Privileges to grant. Granting a held privilege does nothing.
    #[serde(default)]
    pub add: Vec<String>,
    /// Privileges to revoke. Revoking a privilege not held does nothing.
    #[serde(default)]
    pub remove: Vec<String>,
}

/// A group a subject is in, directly or through other groups.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupMembership {
    /// The group's id, as in [`Subject::group`].
    pub id: String,
    /// The group's name, its Cedar entity id.
    pub name: String,
    /// Whether the subject is in it directly.
    pub direct: bool,
    /// The names of the groups this group is directly in.
    pub parents: Vec<String>,
}

/// What a subject holds: its groups and the grants on some objects.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubjectFacts {
    /// Every group the subject is in, transitively.
    pub groups: Vec<GroupMembership>,
    /// The grants the subject or one of its groups holds on the objects asked about.
    pub grants: Vec<Grant>,
}

/// A grant the store refuses, or a store failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GrantError {
    /// A privilege name that is not `[A-Za-z][A-Za-z0-9_]{0,62}`.
    #[error("invalid privilege {0:?}")]
    InvalidPrivilege(String),
    /// An object whose type is not a qualified Cedar type name (`Ns::Type`) or whose id
    /// is empty.
    #[error("invalid object {0}")]
    InvalidObject(String),
    /// A rename onto the object itself or one of its descendants.
    #[error("cannot rename {from} to {to:?}")]
    InvalidRename {
        /// The renamed object.
        from: String,
        /// The new id.
        to: String,
    },
    /// The store failed.
    #[error(transparent)]
    Store(BoxError),
}

/// Checks that `privilege` is a privilege name: a letter, then up to 62 letters, digits
/// and underscores.
pub fn validate_privilege(privilege: &str) -> Result<(), GrantError> {
    let mut chars = privilege.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && privilege.len() <= 63
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if valid {
        Ok(())
    } else {
        Err(GrantError::InvalidPrivilege(privilege.to_owned()))
    }
}

/// Checks that `object` has a qualified Cedar type name (`Ns::Type`, each part an
/// upper-case letter then letters, digits and underscores) and a non-empty id.
pub fn validate_object(object: &ObjectRef) -> Result<(), GrantError> {
    let part = |p: &str| {
        let mut chars = p.chars();
        chars.next().is_some_and(|c| c.is_ascii_uppercase())
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    };
    let parts: Vec<&str> = object.entity_type.split("::").collect();
    if parts.len() >= 2 && parts.iter().all(|p| part(p)) && !object.id.is_empty() {
        Ok(())
    } else {
        Err(GrantError::InvalidObject(object.to_string()))
    }
}

/// Reads and writes [`Grant`]s and resolves group membership.
///
/// Implementations validate privileges ([`validate_privilege`]) and objects
/// ([`validate_object`]) on writes, and apply each write atomically.
#[async_trait]
pub trait GrantStore: Send + Sync + 'static {
    /// The grants on `object` itself, sorted by subject then privilege.
    async fn list(&self, object: &ObjectRef) -> Result<Vec<Grant>, GrantError>;

    /// Applies `changes` to the grants on `object`, all or none, and returns the grants on
    /// it afterwards, as [`list`](Self::list) does.
    async fn apply(
        &self,
        object: &ObjectRef,
        changes: &[GrantChange],
    ) -> Result<Vec<Grant>, GrantError>;

    /// Deletes every grant on `object` and on its descendants of `descendant_types`, as
    /// when the object is deleted. Returns how many grants were deleted.
    async fn delete_object(
        &self,
        object: &ObjectRef,
        descendant_types: &[&str],
    ) -> Result<u64, GrantError>;

    /// Moves every grant on `object` to the object of the same type with id `new_id`, and
    /// those on its descendants of `descendant_types` to the same descendants of the new
    /// id, as when the object is renamed. A grant the destination already holds is kept
    /// once. Returns how many grants were moved.
    async fn rename(
        &self,
        object: &ObjectRef,
        new_id: &str,
        descendant_types: &[&str],
    ) -> Result<u64, GrantError>;

    /// The groups `subject` is in, transitively, sorted by name.
    async fn groups(&self, subject: &Subject) -> Result<Vec<GroupMembership>, GrantError>;

    /// The grants `subject` holds on `objects`, itself or through one of its groups,
    /// sorted. A grant through a group names the group as its subject.
    async fn effective(
        &self,
        subject: &Subject,
        objects: &[ObjectRef],
    ) -> Result<Vec<Grant>, GrantError>;

    /// [`groups`](Self::groups) and [`effective`](Self::effective) at once, what an
    /// entity loader needs per request. Stores that can answer both in one round trip
    /// override it.
    async fn facts(
        &self,
        subject: &Subject,
        objects: &[ObjectRef],
    ) -> Result<SubjectFacts, GrantError> {
        Ok(SubjectFacts {
            groups: self.groups(subject).await?,
            grants: self.effective(subject, objects).await?,
        })
    }
}

/// A shared store is a store, so one can back both a grants API and an entity loader.
#[async_trait]
impl<T: GrantStore + ?Sized> GrantStore for std::sync::Arc<T> {
    async fn list(&self, object: &ObjectRef) -> Result<Vec<Grant>, GrantError> {
        (**self).list(object).await
    }

    async fn apply(
        &self,
        object: &ObjectRef,
        changes: &[GrantChange],
    ) -> Result<Vec<Grant>, GrantError> {
        (**self).apply(object, changes).await
    }

    async fn delete_object(
        &self,
        object: &ObjectRef,
        descendant_types: &[&str],
    ) -> Result<u64, GrantError> {
        (**self).delete_object(object, descendant_types).await
    }

    async fn rename(
        &self,
        object: &ObjectRef,
        new_id: &str,
        descendant_types: &[&str],
    ) -> Result<u64, GrantError> {
        (**self).rename(object, new_id, descendant_types).await
    }

    async fn groups(&self, subject: &Subject) -> Result<Vec<GroupMembership>, GrantError> {
        (**self).groups(subject).await
    }

    async fn effective(
        &self,
        subject: &Subject,
        objects: &[ObjectRef],
    ) -> Result<Vec<Grant>, GrantError> {
        (**self).effective(subject, objects).await
    }

    async fn facts(
        &self,
        subject: &Subject,
        objects: &[ObjectRef],
    ) -> Result<SubjectFacts, GrantError> {
        (**self).facts(subject, objects).await
    }
}

/// Whether `id` is a descendant id of `ancestor`: it starts with `ancestor` and `separator`.
fn is_descendant(id: &str, ancestor: &str, separator: &str) -> bool {
    id.strip_prefix(ancestor)
        .is_some_and(|rest| rest.starts_with(separator) && rest.len() > separator.len())
}

/// Whether a rename of `object` to `new_id` would land on the object or a descendant.
fn check_rename(object: &ObjectRef, new_id: &str, separator: &str) -> Result<(), GrantError> {
    if new_id.is_empty() || new_id == object.id || is_descendant(new_id, &object.id, separator) {
        return Err(GrantError::InvalidRename {
            from: object.to_string(),
            to: new_id.to_owned(),
        });
    }
    Ok(())
}

fn check_changes(object: &ObjectRef, changes: &[GrantChange]) -> Result<(), GrantError> {
    validate_object(object)?;
    for change in changes {
        for privilege in change.add.iter().chain(&change.remove) {
            validate_privilege(privilege)?;
        }
    }
    Ok(())
}

#[derive(Default)]
struct Memory {
    /// `(object, subject, privilege)`, so an object's grants sort by subject.
    grants: BTreeSet<(ObjectRef, Subject, String)>,
    /// Group id to name.
    groups: BTreeMap<String, String>,
    /// Group id to its direct members.
    members: HashMap<String, BTreeSet<Subject>>,
}

impl Memory {
    /// Every group `subject` is in, transitively, with whether directly.
    fn groups_of(&self, subject: &Subject) -> BTreeMap<String, bool> {
        let direct_of = |member: &Subject| -> Vec<String> {
            self.members
                .iter()
                .filter(|(_, members)| members.contains(member))
                .map(|(group, _)| group.clone())
                .collect()
        };
        let mut found: BTreeMap<String, bool> =
            direct_of(subject).into_iter().map(|g| (g, true)).collect();
        let mut pending: Vec<String> = found.keys().cloned().collect();
        while let Some(group) = pending.pop() {
            for parent in direct_of(&Subject::group(group)) {
                if !found.contains_key(&parent) {
                    found.insert(parent.clone(), false);
                    pending.push(parent);
                }
            }
        }
        found
    }

    fn name(&self, group: &str) -> String {
        self.groups
            .get(group)
            .cloned()
            .unwrap_or_else(|| group.to_owned())
    }

    fn list(&self, object: &ObjectRef) -> Vec<Grant> {
        self.grants
            .iter()
            .filter(|(o, _, _)| o == object)
            .map(|(object, subject, privilege)| Grant {
                subject: subject.clone(),
                privilege: privilege.clone(),
                object: object.clone(),
            })
            .collect()
    }
}

/// A [`GrantStore`] in memory, for tests and single-process deployments.
///
/// Groups and memberships are set up with [`add_group`](Self::add_group) and
/// [`add_member`](Self::add_member).
pub struct MemoryGrantStore {
    separator: String,
    state: Mutex<Memory>,
}

impl Default for MemoryGrantStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryGrantStore {
    /// An empty store.
    pub fn new() -> Self {
        Self {
            separator: DEFAULT_SEPARATOR.to_owned(),
            state: Mutex::default(),
        }
    }

    /// Separates the parts of hierarchical object ids with `separator` instead of `.`.
    pub fn with_separator(mut self, separator: impl Into<String>) -> Self {
        self.separator = separator.into();
        self
    }

    /// Adds the group `id` named `name`, or renames it.
    pub fn add_group(&self, id: impl Into<String>, name: impl Into<String>) {
        self.lock().groups.insert(id.into(), name.into());
    }

    /// Makes `member` a direct member of the group `group`.
    pub fn add_member(&self, group: impl Into<String>, member: Subject) {
        self.lock()
            .members
            .entry(group.into())
            .or_default()
            .insert(member);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Memory> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn affected(&self, grant: &ObjectRef, object: &ObjectRef, types: &[&str]) -> bool {
        grant == object
            || (types.contains(&grant.entity_type.as_str())
                && is_descendant(&grant.id, &object.id, &self.separator))
    }
}

#[async_trait]
impl GrantStore for MemoryGrantStore {
    async fn list(&self, object: &ObjectRef) -> Result<Vec<Grant>, GrantError> {
        Ok(self.lock().list(object))
    }

    async fn apply(
        &self,
        object: &ObjectRef,
        changes: &[GrantChange],
    ) -> Result<Vec<Grant>, GrantError> {
        check_changes(object, changes)?;
        let mut state = self.lock();
        for change in changes {
            for privilege in &change.add {
                state
                    .grants
                    .insert((object.clone(), change.subject.clone(), privilege.clone()));
            }
            for privilege in &change.remove {
                state
                    .grants
                    .remove(&(object.clone(), change.subject.clone(), privilege.clone()));
            }
        }
        Ok(state.list(object))
    }

    async fn delete_object(
        &self,
        object: &ObjectRef,
        descendant_types: &[&str],
    ) -> Result<u64, GrantError> {
        let mut state = self.lock();
        let before = state.grants.len();
        state
            .grants
            .retain(|(o, _, _)| !self.affected(o, object, descendant_types));
        Ok((before - state.grants.len()) as u64)
    }

    async fn rename(
        &self,
        object: &ObjectRef,
        new_id: &str,
        descendant_types: &[&str],
    ) -> Result<u64, GrantError> {
        check_rename(object, new_id, &self.separator)?;
        let mut state = self.lock();
        let moved: Vec<_> = state
            .grants
            .iter()
            .filter(|(o, _, _)| self.affected(o, object, descendant_types))
            .cloned()
            .collect();
        for grant in &moved {
            state.grants.remove(grant);
        }
        for (o, subject, privilege) in &moved {
            let id = format!("{new_id}{}", &o.id[object.id.len()..]);
            let renamed = ObjectRef::new(o.entity_type.clone(), id);
            state
                .grants
                .insert((renamed, subject.clone(), privilege.clone()));
        }
        Ok(moved.len() as u64)
    }

    async fn groups(&self, subject: &Subject) -> Result<Vec<GroupMembership>, GrantError> {
        let state = self.lock();
        let mut groups: Vec<GroupMembership> = state
            .groups_of(subject)
            .into_iter()
            .map(|(id, direct)| {
                let mut parents: Vec<String> = state
                    .members
                    .iter()
                    .filter(|(_, members)| members.contains(&Subject::group(id.clone())))
                    .map(|(parent, _)| state.name(parent))
                    .collect();
                parents.sort();
                GroupMembership {
                    name: state.name(&id),
                    id,
                    direct,
                    parents,
                }
            })
            .collect();
        groups.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(groups)
    }

    async fn effective(
        &self,
        subject: &Subject,
        objects: &[ObjectRef],
    ) -> Result<Vec<Grant>, GrantError> {
        let state = self.lock();
        let mut holders: BTreeSet<Subject> = state
            .groups_of(subject)
            .into_keys()
            .map(Subject::group)
            .collect();
        holders.insert(subject.clone());
        let mut grants: Vec<Grant> = state
            .grants
            .iter()
            .filter(|(o, s, _)| objects.contains(o) && holders.contains(s))
            .map(|(object, subject, privilege)| Grant {
                subject: subject.clone(),
                privilege: privilege.clone(),
                object: object.clone(),
            })
            .collect();
        grants.sort();
        Ok(grants)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_shared_store_is_one_store() {
        let shared = std::sync::Arc::new(MemoryGrantStore::new());
        let writer: std::sync::Arc<dyn GrantStore> = shared.clone();
        let doc = ObjectRef::new("Ns::Doc", "d");
        let change = GrantChange {
            subject: Subject::user("u1"),
            add: vec!["READ".into()],
            remove: Vec::new(),
        };
        writer.apply(&doc, &[change]).await.unwrap();
        let grants = GrantStore::list(&shared, &doc).await.unwrap();
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].privilege, "READ");
    }

    #[test]
    fn validates_privileges_and_objects() {
        for ok in ["READ", "USE_SCHEMA", "owner", "a1"] {
            assert!(validate_privilege(ok).is_ok(), "{ok}");
        }
        for bad in ["", "1READ", "READ-ALL", "READ ALL", &"A".repeat(64)] {
            assert!(validate_privilege(bad).is_err(), "{bad}");
        }
        assert!(validate_object(&ObjectRef::new("Ns::Doc", "d")).is_ok());
        assert!(validate_object(&ObjectRef::new("A::B::C_1", "d")).is_ok());
        for (t, id) in [
            ("Doc", "d"),
            ("ns::Doc", "d"),
            ("Ns::", "d"),
            ("Ns::Doc", ""),
        ] {
            assert!(validate_object(&ObjectRef::new(t, id)).is_err(), "{t} {id}");
        }
    }

    #[test]
    fn descendants_need_the_separator() {
        assert!(is_descendant("a.b", "a", "."));
        assert!(is_descendant("a.b.c", "a", "."));
        assert!(!is_descendant("ab", "a", "."));
        assert!(!is_descendant("a.", "a", "."));
        assert!(!is_descendant("a", "a", "."));
        assert!(is_descendant("a/b", "a", "/"));
    }

    #[test]
    fn subject_kinds_round_trip() {
        for kind in [
            SubjectKind::User,
            SubjectKind::ServiceAccount,
            SubjectKind::Group,
        ] {
            assert_eq!(SubjectKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(SubjectKind::parse("role"), None);
    }
}
