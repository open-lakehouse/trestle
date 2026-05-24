//! Walk a template tree and render each file into the destination directory.
//!
//! Rules (see also `docs/templates.md`):
//!
//! - The `template/` subdirectory of a template or component is the only thing
//!   copied into the project.
//! - Both filenames and file contents are rendered with MiniJinja.
//! - Files ending in `.tmpl` have their suffix stripped after rendering, providing
//!   an escape hatch for files that legitimately contain `{{` literals.
//! - A `.trestle-ignore` file at the template root (gitignore syntax) excludes
//!   matching paths.

use std::fs;
use std::path::{Path, PathBuf};

use minijinja::Value;

use crate::error::{Error, Result};

use super::render::Renderer;

/// Render the entire `template/` subtree of one template root into `dest`,
/// using the provided MiniJinja context.
///
/// `dest` must already exist; relative paths inside the template are joined with
/// it. Returns the number of files written.
pub fn render_tree(
    template_root: &Path,
    dest: &Path,
    ctx: &Value,
    renderer: &Renderer,
) -> Result<usize> {
    let tree = template_root.join("template");
    if !tree.is_dir() {
        return Ok(0);
    }

    let ignore_path = template_root.join(".trestle-ignore");
    let ignored = if ignore_path.is_file() {
        let bytes = fs::read(&ignore_path).map_err(|e| Error::io_at(&ignore_path, e))?;
        let s = String::from_utf8_lossy(&bytes).into_owned();
        Some(build_ignore_set(s.as_str(), &tree))
    } else {
        None
    };

    let mut written = 0usize;
    for entry in walkdir::WalkDir::new(&tree).follow_links(false) {
        let entry =
            entry.map_err(|e| Error::other(format!("walkdir error in {}: {e}", tree.display())))?;
        if entry.file_type().is_dir() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(&tree)
            .map_err(|e| Error::other(format!("strip_prefix: {e}")))?;
        if rel.as_os_str().is_empty() {
            continue;
        }

        // Honour .trestle-ignore.
        if let Some(set) = &ignored {
            if set
                .matched(entry.path(), entry.file_type().is_dir())
                .is_ignore()
            {
                continue;
            }
        }

        // Render the relative path.
        let rel_str = rel.to_string_lossy();
        let rendered_rel = renderer.render_str(&rel_str, ctx, &format!("path:{rel_str}"))?;
        if rendered_rel.is_empty() {
            // A path that renders to nothing is conventionally treated as "skip".
            continue;
        }
        let rendered_rel = strip_tmpl_suffix(&rendered_rel);
        let out_path = dest.join(&rendered_rel);

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|e| Error::io_at(parent, e))?;
        }

        let bytes = fs::read(entry.path()).map_err(|e| Error::io_at(entry.path(), e))?;
        let should_render = entry
            .path()
            .extension()
            .map(|e| e == "tmpl")
            .unwrap_or(false)
            || looks_like_text(&bytes);
        if should_render {
            let contents = match std::str::from_utf8(&bytes) {
                Ok(s) => s.to_string(),
                Err(_) => {
                    // Binary file (even if it had .tmpl): copy verbatim.
                    fs::write(&out_path, &bytes).map_err(|e| Error::io_at(&out_path, e))?;
                    written += 1;
                    continue;
                }
            };
            let rendered = renderer.render_str(&contents, ctx, &out_path.to_string_lossy())?;
            fs::write(&out_path, rendered.as_bytes()).map_err(|e| Error::io_at(&out_path, e))?;
        } else {
            fs::write(&out_path, &bytes).map_err(|e| Error::io_at(&out_path, e))?;
        }

        // Preserve unix executable bit (so post-init scripts stay runnable).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = fs::metadata(entry.path()).map_err(|e| Error::io_at(entry.path(), e))?;
            let mode = meta.permissions().mode();
            if mode & 0o111 != 0 {
                let mut perms = fs::metadata(&out_path)
                    .map_err(|e| Error::io_at(&out_path, e))?
                    .permissions();
                perms.set_mode(mode | 0o644);
                fs::set_permissions(&out_path, perms).map_err(|e| Error::io_at(&out_path, e))?;
            }
        }

        written += 1;
    }

    Ok(written)
}

fn strip_tmpl_suffix(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_suffix(".tmpl") {
        PathBuf::from(stripped)
    } else {
        PathBuf::from(path)
    }
}

fn build_ignore_set(spec: &str, root: &Path) -> ignore::gitignore::Gitignore {
    let mut b = ignore::gitignore::GitignoreBuilder::new(root);
    for line in spec.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let _ = b.add_line(None, line);
    }
    b.build()
        .unwrap_or_else(|_| ignore::gitignore::Gitignore::empty())
}

/// Heuristic for whether to attempt MiniJinja rendering of a file. The check looks
/// at the first 4KiB for NUL bytes; if there are none, we treat it as text.
fn looks_like_text(bytes: &[u8]) -> bool {
    let scan = &bytes[..bytes.len().min(4096)];
    !scan.contains(&0u8)
}
