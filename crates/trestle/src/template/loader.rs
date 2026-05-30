//! Resolve a `--template <name|url|path>` argument into a materialised template tree
//! on disk, together with its parsed manifest.
//!
//! Three sources are supported:
//! - **Embedded**: built into the binary via [`rust_embed`]. Materialised into a
//!   freshly-created tempdir on each invocation.
//! - **Git**: cloned with the system `git` binary into `~/.cache/trestle/templates/<sha>`.
//!   Subsequent invocations reuse the clone unless `refresh = true`.
//! - **Local**: a path to a directory on disk; used as-is (read-only).
//!
//! Both top-level templates and shared components share the same on-disk shape and use
//! the same loader; the only difference is the manifest type ([`Manifest`] vs
//! [`ComponentManifest`]).

use std::fs;
use std::path::{Path, PathBuf};

use crate::embedded::{
    APP_TEMPLATES_PREFIX, BASE_TEMPLATES_PREFIX, RustEmbed, SHARED_COMPONENTS_PREFIX, Templates,
    embedded_app_names, embedded_base_names,
};
use crate::error::{Error, Result};

use super::manifest::{ComponentManifest, Manifest};

/// User-supplied template specifier.
#[derive(Debug, Clone)]
pub enum TemplateSource {
    /// Embedded template (e.g. `databricks-app-rust`).
    Embedded(String),
    /// Remote template (https/ssh git URL). Optional `rev` pins a commit/tag/branch.
    Git { url: String, rev: Option<String> },
    /// Filesystem path. Useful for template authors.
    Local(PathBuf),
}

impl TemplateSource {
    /// Auto-detect a source from a user-provided spec.
    ///
    /// * `http(s)://...` or `git@...:.../...` → [`TemplateSource::Git`]
    /// * Existing directory path → [`TemplateSource::Local`]
    /// * Otherwise → [`TemplateSource::Embedded`]
    pub fn detect(spec: &str) -> Self {
        if spec.starts_with("http://")
            || spec.starts_with("https://")
            || spec.starts_with("git@")
            || spec.ends_with(".git")
        {
            return TemplateSource::Git {
                url: spec.to_string(),
                rev: None,
            };
        }
        let p = PathBuf::from(spec);
        if p.is_dir() {
            return TemplateSource::Local(p);
        }
        TemplateSource::Embedded(spec.to_string())
    }
}

/// A template on disk, plus its parsed top-level manifest.
pub struct LoadedTemplate {
    /// Root directory containing `template.yaml`, `template/`, and (optionally)
    /// `components/`. May point at a tempdir for embedded sources.
    pub root: PathBuf,
    pub manifest: Manifest,
    /// Held to keep a tempdir alive for embedded sources. Dropped with the struct.
    _keepalive: Option<tempfile::TempDir>,
}

/// Materialise a [`TemplateSource`] onto disk and parse its manifest.
pub fn load_template(source: &TemplateSource) -> Result<LoadedTemplate> {
    match source {
        TemplateSource::Embedded(name) => load_embedded(name),
        TemplateSource::Local(path) => load_local(path),
        TemplateSource::Git { url, rev } => load_git(url, rev.as_deref()),
    }
}

/// Materialise an embedded shared component into a tempdir and parse its manifest.
///
/// Used during component resolution; not exposed to end users directly.
pub fn load_shared_component(name: &str) -> Result<LoadedComponent> {
    let prefix = format!("{SHARED_COMPONENTS_PREFIX}{name}/");
    if !<Templates as RustEmbed>::iter().any(|p| p.starts_with(&prefix)) {
        return Err(Error::ComponentNotFound {
            name: name.to_string(),
        });
    }
    let tmp = tempfile::tempdir()?;
    extract_embedded_subtree(&prefix, tmp.path())?;
    let manifest = read_component_manifest(tmp.path())?;
    Ok(LoadedComponent {
        root: tmp.path().to_path_buf(),
        manifest,
        _keepalive: Some(tmp),
    })
}

/// A component on disk, plus its parsed manifest.
pub struct LoadedComponent {
    pub root: PathBuf,
    pub manifest: ComponentManifest,
    _keepalive: Option<tempfile::TempDir>,
}

impl LoadedComponent {
    /// Build a synthetic [`LoadedComponent`] without touching the filesystem.
    /// Test-only — production code always goes through one of the `load_*`
    /// entry points so paths are tracked properly.
    #[cfg(test)]
    pub(crate) fn synthetic(root: PathBuf, manifest: ComponentManifest) -> Self {
        Self {
            root,
            manifest,
            _keepalive: None,
        }
    }
}

/// Load a component from a path under the parent template's `components/` directory.
pub fn load_local_component(root: PathBuf) -> Result<LoadedComponent> {
    let manifest = read_component_manifest(&root)?;
    Ok(LoadedComponent {
        root,
        manifest,
        _keepalive: None,
    })
}

fn load_embedded(name: &str) -> Result<LoadedTemplate> {
    let prefix = resolve_embedded_template_prefix(name)?;
    let tmp = tempfile::tempdir()?;
    extract_embedded_subtree(&prefix, tmp.path())?;
    let manifest = read_template_manifest(tmp.path())?;
    Ok(LoadedTemplate {
        root: tmp.path().to_path_buf(),
        manifest,
        _keepalive: Some(tmp),
    })
}

/// Map a user-friendly embedded name (e.g. `lakehouse` or `databricks-app-rust`)
/// onto its prefix inside the embedded asset bundle. Bases live under `_base/`
/// and apps under `_apps/`; both can be referenced by short name.
fn resolve_embedded_template_prefix(name: &str) -> Result<String> {
    let candidates = [
        format!("{BASE_TEMPLATES_PREFIX}{name}/"),
        format!("{APP_TEMPLATES_PREFIX}{name}/"),
    ];
    for prefix in &candidates {
        if <Templates as RustEmbed>::iter().any(|p| p.starts_with(prefix.as_str())) {
            return Ok(prefix.clone());
        }
    }
    Err(Error::TemplateNotFound {
        name: name.to_string(),
    })
}

/// Embedded base template (e.g. `lakehouse`).
pub fn load_embedded_base(name: &str) -> Result<LoadedTemplate> {
    let prefix = format!("{BASE_TEMPLATES_PREFIX}{name}/");
    if !<Templates as RustEmbed>::iter().any(|p| p.starts_with(prefix.as_str())) {
        return Err(Error::TemplateNotFound {
            name: name.to_string(),
        });
    }
    let tmp = tempfile::tempdir()?;
    extract_embedded_subtree(&prefix, tmp.path())?;
    let manifest = read_template_manifest(tmp.path())?;
    Ok(LoadedTemplate {
        root: tmp.path().to_path_buf(),
        manifest,
        _keepalive: Some(tmp),
    })
}

/// Embedded app template (e.g. `databricks-app-rust`).
pub fn load_embedded_app(name: &str) -> Result<LoadedTemplate> {
    let prefix = format!("{APP_TEMPLATES_PREFIX}{name}/");
    if !<Templates as RustEmbed>::iter().any(|p| p.starts_with(prefix.as_str())) {
        return Err(Error::TemplateNotFound {
            name: name.to_string(),
        });
    }
    let tmp = tempfile::tempdir()?;
    extract_embedded_subtree(&prefix, tmp.path())?;
    let manifest = read_template_manifest(tmp.path())?;
    Ok(LoadedTemplate {
        root: tmp.path().to_path_buf(),
        manifest,
        _keepalive: Some(tmp),
    })
}

/// Iterate over the names of all known embedded base templates.
pub fn list_embedded_bases() -> Vec<String> {
    embedded_base_names()
}

/// Iterate over the names of all known embedded apps.
pub fn list_embedded_apps() -> Vec<String> {
    embedded_app_names()
}

fn load_local(path: &Path) -> Result<LoadedTemplate> {
    let manifest = read_template_manifest(path)?;
    Ok(LoadedTemplate {
        root: path.to_path_buf(),
        manifest,
        _keepalive: None,
    })
}

fn load_git(url: &str, rev: Option<&str>) -> Result<LoadedTemplate> {
    let cache_root = dirs::cache_dir()
        .ok_or_else(|| Error::other("could not locate user cache dir"))?
        .join("trestle/templates");
    fs::create_dir_all(&cache_root).map_err(|e| Error::io_at(&cache_root, e))?;

    let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
    sha2::Digest::update(&mut hasher, url.as_bytes());
    if let Some(r) = rev {
        sha2::Digest::update(&mut hasher, b"@");
        sha2::Digest::update(&mut hasher, r.as_bytes());
    }
    let digest = sha2::Digest::finalize(hasher);
    let dir = cache_root.join(hex::encode(digest));

    if !dir.exists() {
        run_git(&["clone", "--depth", "1", url, dir.to_string_lossy().as_ref()])?;
        if let Some(r) = rev {
            run_git_in(&dir, &["fetch", "--depth", "1", "origin", r])?;
            run_git_in(&dir, &["checkout", r])?;
        }
    }

    let manifest = read_template_manifest(&dir)?;
    Ok(LoadedTemplate {
        root: dir,
        manifest,
        _keepalive: None,
    })
}

fn run_git(args: &[&str]) -> Result<()> {
    let status = std::process::Command::new("git")
        .args(args)
        .status()
        .map_err(|e| Error::Git(format!("failed to spawn git: {e}")))?;
    if !status.success() {
        return Err(Error::Git(format!(
            "git {} failed with status {status}",
            args.join(" ")
        )));
    }
    Ok(())
}

fn run_git_in(dir: &Path, args: &[&str]) -> Result<()> {
    let status = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .map_err(|e| Error::Git(format!("failed to spawn git: {e}")))?;
    if !status.success() {
        return Err(Error::Git(format!(
            "git {} failed with status {status}",
            args.join(" ")
        )));
    }
    Ok(())
}

fn read_template_manifest(root: &Path) -> Result<Manifest> {
    let path = root.join("template.yaml");
    let bytes = fs::read(&path).map_err(|e| Error::io_at(&path, e))?;
    let m: Manifest = serde_yaml::from_slice(&bytes).map_err(|e| Error::yaml_at(&path, e))?;
    if m.name.is_empty() {
        return Err(Error::Manifest(format!(
            "{}: top-level `name` is required",
            path.display()
        )));
    }
    Ok(m)
}

fn read_component_manifest(root: &Path) -> Result<ComponentManifest> {
    let path = root.join("template.yaml");
    let bytes = fs::read(&path).map_err(|e| Error::io_at(&path, e))?;
    let m: ComponentManifest =
        serde_yaml::from_slice(&bytes).map_err(|e| Error::yaml_at(&path, e))?;
    if m.name.is_empty() {
        return Err(Error::Manifest(format!(
            "{}: component `name` is required",
            path.display()
        )));
    }
    Ok(m)
}

/// Copy a subtree out of the embedded asset bundle into a real directory.
fn extract_embedded_subtree(prefix: &str, dest: &Path) -> Result<()> {
    for path in <Templates as RustEmbed>::iter() {
        let Some(rel) = path.strip_prefix(prefix) else {
            continue;
        };
        if rel.is_empty() {
            continue;
        }
        let target = dest.join(rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| Error::io_at(parent, e))?;
        }
        let file = <Templates as RustEmbed>::get(&path)
            .ok_or_else(|| Error::other(format!("embedded asset disappeared mid-load: {path}")))?;
        fs::write(&target, file.data.as_ref()).map_err(|e| Error::io_at(&target, e))?;
    }
    Ok(())
}
