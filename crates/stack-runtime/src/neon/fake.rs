//! [`FakeNeon`]: an in-memory [`NeonControl`] for tests (feature `testing`).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;

use super::cli::{ApiBranches, first_json};
use super::{
    Branch, Connection, CreateBranch, Function, NeonControl, NeonError, Storage, StorageCredential,
};
use crate::secrets::SecretValue;

/// A call [`FakeNeon`] was asked to make.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NeonCall {
    /// [`create_branch`](NeonControl::create_branch) named this.
    CreateBranch(String),
    /// [`delete_branch`](NeonControl::delete_branch) of this id.
    DeleteBranch(String),
    /// [`reset_branch`](NeonControl::reset_branch) of this id.
    ResetBranch(String),
    /// [`create_database`](NeonControl::create_database): branch id, name.
    CreateDatabase(String, String),
    /// [`create_bucket`](NeonControl::create_bucket): branch id, name.
    CreateBucket(String, String),
    /// [`create_credential`](NeonControl::create_credential): branch id, token id.
    CreateCredential(String, String),
    /// [`revoke_credential`](NeonControl::revoke_credential): branch id, token id.
    RevokeCredential(String, String),
}

#[derive(Clone, Default)]
struct FakeBranch {
    branch: Branch,
    functions: Vec<Function>,
    databases: Vec<String>,
    buckets: Vec<String>,
    /// Token id → secret.
    credentials: BTreeMap<String, String>,
}

#[derive(Default)]
struct Inner {
    project: String,
    branches: Vec<FakeBranch>,
    calls: Vec<NeonCall>,
    signed_in: bool,
    next: u32,
}

/// One Neon project in memory, behaving as the spike found the real one does (`docs/neon.md`):
/// a new branch copies its parent's functions, buckets and credentials at creation, a reset
/// leaves functions alone, and annotations are kept. Every change is recorded
/// ([`calls`](Self::calls)).
#[derive(Clone)]
pub struct FakeNeon {
    inner: Arc<Mutex<Inner>>,
}

impl FakeNeon {
    /// The project `project` with one default branch, `main` (database `neondb`, storage
    /// enabled), signed in.
    pub fn new(project: &str) -> Self {
        let main = FakeBranch {
            branch: Branch {
                id: "br-main".into(),
                name: "main".into(),
                parent_id: None,
                expires_at: None,
                default: true,
                annotations: BTreeMap::new(),
            },
            databases: vec!["neondb".into()],
            ..FakeBranch::default()
        };
        FakeNeon {
            inner: Arc::new(Mutex::new(Inner {
                project: project.to_owned(),
                branches: vec![main],
                signed_in: true,
                ..Inner::default()
            })),
        }
    }

    /// The project as the spike captured it in `dir` (`tests/fixtures/neon/<version>/`): its
    /// branches with their annotations (`api-branches.json`, `branches-list.json`) and the
    /// functions on the default branch (`functions-list.json`).
    pub fn from_fixtures(project: &str, dir: &Path) -> Result<Self, String> {
        let read = |name: &str| {
            std::fs::read_to_string(dir.join(name)).map_err(|e| format!("{name}: {e}"))
        };
        let listed: Vec<Branch> = first_json(&read("branches-list.json")?)?;
        let annotated: ApiBranches = first_json(&read("api-branches.json")?)?;
        let annotated = annotated.into_branches();
        let functions: Vec<Function> = first_json(&read("functions-list.json")?)?;
        let fake = FakeNeon::new(project);
        {
            let mut inner = fake.lock();
            inner.branches = listed
                .into_iter()
                .map(|b| {
                    let annotations = annotated
                        .iter()
                        .find(|a| a.id == b.id)
                        .map(|a| a.annotations.clone())
                        .unwrap_or_default();
                    FakeBranch {
                        functions: if b.default {
                            functions.clone()
                        } else {
                            Vec::new()
                        },
                        databases: vec!["neondb".into()],
                        branch: Branch { annotations, ..b },
                        ..FakeBranch::default()
                    }
                })
                .collect();
        }
        Ok(fake)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Deploy the function `slug` on the branch `name_or_id`.
    pub fn deploy_function(&self, name_or_id: &str, slug: &str) {
        let mut inner = self.lock();
        if let Some(b) = inner.branches.iter_mut().find(|b| b.branch.is(name_or_id)) {
            let url = format!("https://{}-{slug}.compute.fake.neon.tech/", b.branch.id);
            b.functions.retain(|f| f.slug != slug);
            b.functions.push(Function {
                slug: slug.to_owned(),
                invocation_url: url,
            });
        }
    }

    /// Sign the CLI out (or back in).
    pub fn set_signed_in(&self, signed_in: bool) {
        self.lock().signed_in = signed_in;
    }

    /// Every change asked of it, in order.
    pub fn calls(&self) -> Vec<NeonCall> {
        self.lock().calls.clone()
    }

    /// The credentials on the branch `name_or_id`, by token id.
    pub fn credentials(&self, name_or_id: &str) -> Vec<String> {
        self.lock()
            .branches
            .iter()
            .find(|b| b.branch.is(name_or_id))
            .map(|b| b.credentials.keys().cloned().collect())
            .unwrap_or_default()
    }

    fn with_branch<T>(
        &self,
        project: &str,
        name_or_id: &str,
        f: impl FnOnce(&mut FakeBranch, &mut Vec<NeonCall>) -> T,
    ) -> Result<T, NeonError> {
        let mut inner = self.lock();
        check(&inner, project)?;
        let Inner {
            branches, calls, ..
        } = &mut *inner;
        let b = branches
            .iter_mut()
            .find(|b| b.branch.is(name_or_id))
            .ok_or_else(|| NeonError::NoSuchBranch(name_or_id.to_owned()))?;
        Ok(f(b, calls))
    }
}

fn check(inner: &Inner, project: &str) -> Result<(), NeonError> {
    if !inner.signed_in {
        return Err(NeonError::NotAuthenticated("signed out".into()));
    }
    if inner.project != project {
        return Err(NeonError::Failed {
            command: format!("--project-id {project}"),
            stderr: "project not found".into(),
        });
    }
    Ok(())
}

#[async_trait]
impl NeonControl for FakeNeon {
    async fn auth_status(&self) -> Result<String, NeonError> {
        if self.lock().signed_in {
            Ok("fake-user".into())
        } else {
            Err(NeonError::NotAuthenticated("signed out".into()))
        }
    }

    async fn branches(&self, project: &str) -> Result<Vec<Branch>, NeonError> {
        let inner = self.lock();
        check(&inner, project)?;
        Ok(inner.branches.iter().map(|b| b.branch.clone()).collect())
    }

    async fn create_branch(&self, project: &str, spec: &CreateBranch) -> Result<Branch, NeonError> {
        let mut inner = self.lock();
        check(&inner, project)?;
        let parent = match &spec.parent {
            Some(p) => inner.branches.iter().find(|b| b.branch.is(p)),
            None => inner.branches.iter().find(|b| b.branch.default),
        }
        .cloned()
        .ok_or_else(|| NeonError::NoSuchBranch(spec.parent.clone().unwrap_or_default()))?;
        if inner.branches.iter().any(|b| b.branch.name == spec.name) {
            return Err(NeonError::Failed {
                command: "branches create".into(),
                stderr: format!("branch {} already exists", spec.name),
            });
        }
        inner.next += 1;
        let id = format!("br-fake-{}", inner.next);
        // A child gets a copy of the parent's functions (each under its own URL), buckets and
        // credentials, as the spike found.
        let child = FakeBranch {
            branch: Branch {
                id: id.clone(),
                name: spec.name.clone(),
                parent_id: Some(parent.branch.id.clone()),
                expires_at: spec.expires_at.clone(),
                default: false,
                annotations: spec.annotations.clone(),
            },
            functions: parent
                .functions
                .iter()
                .map(|f| Function {
                    slug: f.slug.clone(),
                    invocation_url: format!("https://{id}-{}.compute.fake.neon.tech/", f.slug),
                })
                .collect(),
            databases: parent.databases.clone(),
            buckets: parent.buckets.clone(),
            credentials: parent.credentials.clone(),
        };
        let branch = child.branch.clone();
        inner.branches.push(child);
        inner.calls.push(NeonCall::CreateBranch(spec.name.clone()));
        Ok(branch)
    }

    async fn delete_branch(&self, project: &str, id: &str) -> Result<(), NeonError> {
        let mut inner = self.lock();
        check(&inner, project)?;
        let before = inner.branches.len();
        inner.branches.retain(|b| b.branch.id != id);
        if inner.branches.len() == before {
            return Err(NeonError::NoSuchBranch(id.to_owned()));
        }
        inner.calls.push(NeonCall::DeleteBranch(id.to_owned()));
        Ok(())
    }

    async fn reset_branch(&self, project: &str, id: &str) -> Result<(), NeonError> {
        self.with_branch(project, id, |_, calls| {
            calls.push(NeonCall::ResetBranch(id.to_owned()))
        })
    }

    async fn connection(
        &self,
        project: &str,
        branch: &str,
        database: Option<&str>,
        pooled: bool,
    ) -> Result<Connection, NeonError> {
        self.with_branch(project, branch, |b, _| {
            let id = b.branch.id.trim_start_matches("br-");
            let host = format!(
                "ep-{id}{}.c-8.eu-central-1.aws.neon.tech",
                if pooled { "-pooler" } else { "" }
            );
            let database = database.unwrap_or("neondb").to_owned();
            let password = format!("fake-password-{id}");
            let options = "sslmode=require&channel_binding=require".to_owned();
            let userinfo = ["neondb_owner", password.as_str()].join(":");
            Connection {
                url: SecretValue::new(format!(
                    "postgresql://{userinfo}@{host}/{database}?{options}"
                )),
                host,
                role: "neondb_owner".into(),
                password: SecretValue::new(password),
                database,
                options,
            }
        })
    }

    async fn databases(&self, project: &str, branch: &str) -> Result<Vec<String>, NeonError> {
        self.with_branch(project, branch, |b, _| b.databases.clone())
    }

    async fn create_database(
        &self,
        project: &str,
        branch: &str,
        name: &str,
    ) -> Result<(), NeonError> {
        self.with_branch(project, branch, |b, calls| {
            b.databases.push(name.to_owned());
            calls.push(NeonCall::CreateDatabase(
                b.branch.id.clone(),
                name.to_owned(),
            ));
        })
    }

    async fn functions(&self, project: &str, branch: &str) -> Result<Vec<Function>, NeonError> {
        self.with_branch(project, branch, |b, _| b.functions.clone())
    }

    async fn storage(&self, project: &str, branch: &str) -> Result<Storage, NeonError> {
        self.with_branch(project, branch, |b, _| Storage {
            enabled: true,
            s3_endpoint: format!(
                "https://{}.storage.c-8.eu-central-1.aws.neon.tech",
                b.branch.id
            ),
            region: "eu-central-1".into(),
            force_path_style: true,
        })
    }

    async fn buckets(&self, project: &str, branch: &str) -> Result<Vec<String>, NeonError> {
        self.with_branch(project, branch, |b, _| b.buckets.clone())
    }

    async fn create_bucket(
        &self,
        project: &str,
        branch: &str,
        name: &str,
    ) -> Result<(), NeonError> {
        self.with_branch(project, branch, |b, calls| {
            b.buckets.push(name.to_owned());
            calls.push(NeonCall::CreateBucket(b.branch.id.clone(), name.to_owned()));
        })
    }

    async fn create_credential(
        &self,
        project: &str,
        branch: &str,
        name: &str,
    ) -> Result<StorageCredential, NeonError> {
        let n = {
            let mut inner = self.lock();
            inner.next += 1;
            inner.next
        };
        self.with_branch(project, branch, |b, calls| {
            let token_id = format!("nak_fake_{n:032}");
            let secret = format!("fake-s3-secret-{name}-{n}");
            b.credentials.insert(token_id.clone(), secret.clone());
            calls.push(NeonCall::CreateCredential(
                b.branch.id.clone(),
                token_id.clone(),
            ));
            StorageCredential {
                token_id,
                secret_access_key: SecretValue::new(secret),
            }
        })
    }

    async fn revoke_credential(
        &self,
        project: &str,
        branch: &str,
        token_id: &str,
    ) -> Result<(), NeonError> {
        self.with_branch(project, branch, |b, calls| {
            b.credentials.remove(token_id);
            calls.push(NeonCall::RevokeCredential(
                b.branch.id.clone(),
                token_id.to_owned(),
            ));
        })
    }
}
