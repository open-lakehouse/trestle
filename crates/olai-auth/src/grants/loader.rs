use std::collections::{BTreeSet, HashMap, HashSet};

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use super::{GrantStore, ObjectRef, Subject, SubjectFacts, SubjectKind};
use crate::authz::cedar::EntityLoader;
use crate::error::BoxError;
use crate::wire::{AuthorizeRequest, EntityRef, entity_ref};

/// The Cedar entity types of a service's subjects: users, service accounts and groups,
/// such as `Ns::User`, `Ns::ServiceAccount` and `Ns::Group`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectTypes {
    user: String,
    service_account: String,
    group: String,
}

impl SubjectTypes {
    /// Users are entities of type `user`, service accounts of `service_account`, and
    /// groups of `group`.
    pub fn new(
        user: impl Into<String>,
        service_account: impl Into<String>,
        group: impl Into<String>,
    ) -> Self {
        Self {
            user: user.into(),
            service_account: service_account.into(),
            group: group.into(),
        }
    }

    /// The subject a principal entity is: a user or a service account, by its id.
    pub fn subject(&self, principal: &EntityRef) -> Option<Subject> {
        let kind = if principal.entity_type == self.user {
            SubjectKind::User
        } else if principal.entity_type == self.service_account {
            SubjectKind::ServiceAccount
        } else {
            return None;
        };
        Some(Subject::new(kind, principal.id.clone()))
    }

    /// The group entity named `name`.
    pub fn group(&self, name: impl Into<String>) -> EntityRef {
        EntityRef::new(self.group.clone(), name)
    }
}

/// What the host knows about a principal besides its groups.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PrincipalFacts {
    /// The principal entity's attributes, e.g. a role or a ban.
    pub attrs: Map<String, Value>,
    /// More parents of the principal entity, besides its groups, e.g. a role entity.
    pub parents: Vec<EntityRef>,
    /// More entities to add, e.g. the role entity.
    pub entities: Vec<Value>,
}

/// The host's part of a principal entity: attributes from its identity provider and any
/// further parents. Implemented by the host; `()` adds nothing.
#[async_trait]
pub trait PrincipalAttrs: Send + Sync + 'static {
    /// The facts of `principal`, the [`Subject`] `subject`. An error is a deny.
    async fn principal(
        &self,
        principal: &EntityRef,
        subject: &Subject,
    ) -> Result<PrincipalFacts, BoxError>;
}

#[async_trait]
impl PrincipalAttrs for () {
    async fn principal(&self, _: &EntityRef, _: &Subject) -> Result<PrincipalFacts, BoxError> {
        Ok(PrincipalFacts::default())
    }
}

/// An [`EntityLoader`] that loads a request's principal, its groups and the grants it
/// holds from a [`GrantStore`].
///
/// For a request it loads:
///
/// - the principal (a user or service account, see [`SubjectTypes`]) with the
///   attributes and extra parents the host's [`PrincipalAttrs`] adds, and the groups it
///   is directly in as parents;
/// - every group it is in, transitively, by name, with its parent groups;
/// - for the resource and every caller-supplied entity and parent
///   ([`AuthorizeRequest::entity_uids`]), other than the principal and groups, the grants
///   the principal holds on it, itself or through a group, as two attributes:
///   - `privileges`: the privileges held on that entity itself;
///   - `inherited_privileges`: those held on it or on any of its ancestors, by the
///     parents the caller supplied.
///
/// Cedar can't quantify over a set of grants, so the loader resolves them for the
/// request at hand. A policy that needs a privilege on a specific container reads it
/// there (`resource.folder.privileges.contains("USE")`, with `folder` an attribute the
/// caller supplies), and one that a grant on any ancestor satisfies reads
/// `resource.inherited_privileges`.
///
/// So every entity type the caller supplies, or names as a parent, must declare both
/// attributes in the schema:
///
/// ```cedarschema
/// entity Folder { privileges: Set<String>, inherited_privileges: Set<String> };
/// ```
///
/// [`CedarAuthorizer`](crate::CedarAuthorizer) keeps the caller from setting them
/// itself ([`DEFAULT_LOADER_ATTRS`](crate::DEFAULT_LOADER_ATTRS)). A batch asks the store
/// once per distinct principal.
pub struct GrantsEntityLoader<S, P = ()> {
    store: S,
    principals: P,
    types: SubjectTypes,
}

impl<S: GrantStore> GrantsEntityLoader<S> {
    /// Loads grants and groups from `store`, the subjects typed by `types`.
    pub fn new(store: S, types: SubjectTypes) -> Self {
        Self {
            store,
            principals: (),
            types,
        }
    }
}

impl<S: GrantStore, P: PrincipalAttrs> GrantsEntityLoader<S, P> {
    /// Adds the host's facts about the principal.
    pub fn with_principal_attrs<Q: PrincipalAttrs>(
        self,
        principals: Q,
    ) -> GrantsEntityLoader<S, Q> {
        GrantsEntityLoader {
            store: self.store,
            principals,
            types: self.types,
        }
    }

    /// The store grants are loaded from.
    pub fn store(&self) -> &S {
        &self.store
    }

    fn subject(&self, request: &AuthorizeRequest) -> Result<Subject, BoxError> {
        self.types
            .subject(&request.principal)
            .ok_or_else(|| format!("{} is not a user or service account", request.principal).into())
    }

    /// The entities a request's grants are resolved on.
    fn objects(&self, request: &AuthorizeRequest) -> Vec<ObjectRef> {
        request
            .entity_uids()
            .into_iter()
            .filter(|uid| *uid != request.principal && uid.entity_type != self.types.group)
            .collect()
    }

    async fn load_for(
        &self,
        subject: &Subject,
        requests: &[&AuthorizeRequest],
    ) -> Result<(SubjectFacts, PrincipalFacts), BoxError> {
        let mut objects: Vec<ObjectRef> = Vec::new();
        for request in requests {
            for object in self.objects(request) {
                if !objects.contains(&object) {
                    objects.push(object);
                }
            }
        }
        let facts = self.store.facts(subject, &objects).await?;
        let principal = self
            .principals
            .principal(&requests[0].principal, subject)
            .await?;
        Ok((facts, principal))
    }

    fn entities(
        &self,
        request: &AuthorizeRequest,
        facts: &SubjectFacts,
        principal: &PrincipalFacts,
    ) -> Value {
        let mut parents: Vec<EntityRef> = facts
            .groups
            .iter()
            .filter(|g| g.direct)
            .map(|g| self.types.group(g.name.clone()))
            .collect();
        parents.extend(principal.parents.iter().cloned());
        let mut entities = vec![json!({
            "uid": request.principal,
            "attrs": principal.attrs,
            "parents": parents,
        })];
        entities.extend(principal.entities.iter().cloned());
        for group in &facts.groups {
            entities.push(json!({
                "uid": self.types.group(group.name.clone()),
                "attrs": {},
                "parents": group
                    .parents
                    .iter()
                    .map(|p| self.types.group(p.clone()))
                    .collect::<Vec<_>>(),
            }));
        }

        let mut held: HashMap<&ObjectRef, BTreeSet<&str>> = HashMap::new();
        for grant in &facts.grants {
            held.entry(&grant.object)
                .or_default()
                .insert(grant.privilege.as_str());
        }
        let graph = parent_graph(request);
        for object in self.objects(request) {
            let direct = held.get(&object).cloned().unwrap_or_default();
            let mut inherited = direct.clone();
            for ancestor in ancestors(&graph, &object) {
                if let Some(privileges) = held.get(&ancestor) {
                    inherited.extend(privileges.iter().copied());
                }
            }
            entities.push(json!({
                "uid": object,
                "attrs": { "privileges": direct, "inherited_privileges": inherited },
                "parents": [],
            }));
        }
        Value::Array(entities)
    }
}

/// The parents of each caller-supplied entity.
fn parent_graph(request: &AuthorizeRequest) -> HashMap<EntityRef, Vec<EntityRef>> {
    let mut graph: HashMap<EntityRef, Vec<EntityRef>> = HashMap::new();
    for entity in &request.entities {
        let Some(uid) = entity.get("uid").and_then(entity_ref) else {
            continue;
        };
        let parents = entity
            .get("parents")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(entity_ref);
        graph.entry(uid).or_default().extend(parents);
    }
    graph
}

/// Every ancestor of `of` in `graph`, transitively.
fn ancestors(graph: &HashMap<EntityRef, Vec<EntityRef>>, of: &EntityRef) -> HashSet<EntityRef> {
    let mut seen = HashSet::new();
    let mut pending: Vec<&EntityRef> = graph.get(of).into_iter().flatten().collect();
    while let Some(next) = pending.pop() {
        if next != of && seen.insert(next.clone()) {
            pending.extend(graph.get(next).into_iter().flatten());
        }
    }
    seen
}

#[async_trait]
impl<S: GrantStore, P: PrincipalAttrs> EntityLoader for GrantsEntityLoader<S, P> {
    async fn load(&self, request: &AuthorizeRequest) -> Result<Value, BoxError> {
        let subject = self.subject(request)?;
        let (facts, principal) = self.load_for(&subject, &[request]).await?;
        Ok(self.entities(request, &facts, &principal))
    }

    /// Asks the store once per distinct principal in the batch.
    async fn load_batch(&self, requests: &[AuthorizeRequest]) -> Vec<Result<Value, BoxError>> {
        let mut by_principal: Vec<(Subject, Vec<usize>)> = Vec::new();
        let mut results: Vec<Option<Result<Value, BoxError>>> =
            (0..requests.len()).map(|_| None).collect();
        for (i, request) in requests.iter().enumerate() {
            match self.subject(request) {
                Ok(subject) => match by_principal.iter_mut().find(|(s, _)| *s == subject) {
                    Some((_, indices)) => indices.push(i),
                    None => by_principal.push((subject, vec![i])),
                },
                Err(e) => results[i] = Some(Err(e)),
            }
        }
        for (subject, indices) in by_principal {
            let batch: Vec<&AuthorizeRequest> = indices.iter().map(|&i| &requests[i]).collect();
            match self.load_for(&subject, &batch).await {
                Ok((facts, principal)) => {
                    for &i in &indices {
                        results[i] = Some(Ok(self.entities(&requests[i], &facts, &principal)));
                    }
                }
                Err(e) => {
                    let message = e.to_string();
                    for &i in &indices {
                        results[i] = Some(Err(message.clone().into()));
                    }
                }
            }
        }
        results
            .into_iter()
            .map(|r| r.unwrap_or_else(|| Err("not loaded".into())))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grants::{GrantChange, MemoryGrantStore};

    fn types() -> SubjectTypes {
        SubjectTypes::new("Ns::User", "Ns::ServiceAccount", "Ns::Group")
    }

    fn folder(id: &str) -> EntityRef {
        EntityRef::new("Ns::Folder", id)
    }

    async fn grant(
        store: &MemoryGrantStore,
        subject: Subject,
        privilege: &str,
        object: &EntityRef,
    ) {
        store
            .apply(
                object,
                &[GrantChange {
                    subject,
                    add: vec![privilege.into()],
                    remove: vec![],
                }],
            )
            .await
            .unwrap();
    }

    fn request() -> AuthorizeRequest {
        AuthorizeRequest::new(
            EntityRef::new("Ns::User", "u1"),
            EntityRef::new("Ns::Action", "read"),
            EntityRef::new("Ns::Doc", "root.a.doc"),
        )
        .with_entities([
            json!({
                "uid": { "type": "Ns::Doc", "id": "root.a.doc" },
                "attrs": { "folder": { "__entity": folder("root.a") } },
                "parents": [folder("root.a"), folder("root")],
            }),
            json!({ "uid": folder("root.a"), "attrs": {}, "parents": [folder("root")] }),
        ])
    }

    #[tokio::test]
    async fn loads_the_principal_its_groups_and_its_privileges_per_entity() {
        let store = MemoryGrantStore::new();
        store.add_group("g-staff", "staff");
        store.add_group("g-eng", "eng");
        store.add_member("g-staff", Subject::group("g-eng"));
        store.add_member("g-eng", Subject::user("u1"));
        grant(&store, Subject::user("u1"), "USE", &folder("root.a")).await;
        grant(&store, Subject::group("g-staff"), "READ", &folder("root")).await;
        grant(&store, Subject::user("u2"), "WRITE", &folder("root.a")).await;

        let loader = GrantsEntityLoader::new(store, types());
        let entities = loader.load(&request()).await.unwrap();
        let entities = entities.as_array().unwrap();
        let by_uid = |t: &str, id: &str| {
            entities
                .iter()
                .find(|e| e["uid"] == json!({ "type": t, "id": id }))
                .unwrap_or_else(|| panic!("no {t} {id} in {entities:?}"))
        };
        assert_eq!(
            by_uid("Ns::User", "u1")["parents"],
            json!([{ "type": "Ns::Group", "id": "eng" }])
        );
        assert_eq!(
            by_uid("Ns::Group", "eng")["parents"],
            json!([{ "type": "Ns::Group", "id": "staff" }])
        );
        assert_eq!(by_uid("Ns::Group", "staff")["parents"], json!([]));
        assert_eq!(
            by_uid("Ns::Doc", "root.a.doc")["attrs"],
            json!({ "privileges": [], "inherited_privileges": ["READ", "USE"] })
        );
        assert_eq!(
            by_uid("Ns::Folder", "root.a")["attrs"],
            json!({ "privileges": ["USE"], "inherited_privileges": ["READ", "USE"] })
        );
        assert_eq!(
            by_uid("Ns::Folder", "root")["attrs"],
            json!({ "privileges": ["READ"], "inherited_privileges": ["READ"] })
        );
    }

    #[tokio::test]
    async fn adds_the_hosts_principal_facts() {
        struct Roles;
        #[async_trait]
        impl PrincipalAttrs for Roles {
            async fn principal(
                &self,
                _: &EntityRef,
                subject: &Subject,
            ) -> Result<PrincipalFacts, BoxError> {
                let role = EntityRef::new("Ns::Role", "admin");
                Ok(PrincipalFacts {
                    attrs: json!({ "sub": subject.id }).as_object().unwrap().clone(),
                    parents: vec![role.clone()],
                    entities: vec![json!({ "uid": role, "attrs": {}, "parents": [] })],
                })
            }
        }
        let loader =
            GrantsEntityLoader::new(MemoryGrantStore::new(), types()).with_principal_attrs(Roles);
        let entities = loader.load(&request()).await.unwrap();
        assert_eq!(
            entities[0],
            json!({
                "uid": { "type": "Ns::User", "id": "u1" },
                "attrs": { "sub": "u1" },
                "parents": [{ "type": "Ns::Role", "id": "admin" }],
            })
        );
        assert_eq!(
            entities[1]["uid"],
            json!({ "type": "Ns::Role", "id": "admin" })
        );
    }

    #[tokio::test]
    async fn batches_per_principal_and_refuses_other_principals() {
        let store = MemoryGrantStore::new();
        grant(&store, Subject::user("u1"), "READ", &folder("root")).await;
        grant(&store, Subject::user("u2"), "WRITE", &folder("root")).await;
        let loader = GrantsEntityLoader::new(store, types());
        let mut other = request();
        other.principal = EntityRef::new("Ns::User", "u2");
        let mut robot = request();
        robot.principal = EntityRef::new("Ns::Robot", "r1");
        let loaded = loader
            .load_batch(&[request(), other, robot, request()])
            .await;
        assert_eq!(loaded.len(), 4);
        let root = |entities: &Value| {
            entities
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["uid"] == json!(folder("root")))
                .unwrap()["attrs"]["privileges"]
                .clone()
        };
        assert_eq!(root(loaded[0].as_ref().unwrap()), json!(["READ"]));
        assert_eq!(root(loaded[1].as_ref().unwrap()), json!(["WRITE"]));
        let error = loaded[2].as_ref().unwrap_err().to_string();
        assert!(error.contains("not a user or service account"), "{error}");
        assert_eq!(root(loaded[3].as_ref().unwrap()), json!(["READ"]));
    }

    #[test]
    fn ancestors_survive_cycles() {
        let mut graph = HashMap::new();
        graph.insert(folder("a"), vec![folder("b")]);
        graph.insert(folder("b"), vec![folder("a"), folder("c")]);
        let found = ancestors(&graph, &folder("a"));
        assert_eq!(found, HashSet::from([folder("b"), folder("c")]));
    }
}
