use std::fs;
use std::path::Path;

use crate::Result;
use crate::codegen::GeneratedCode;

/// Return the appropriate "do not edit" header for a given file path.
///
/// Python uses `#` comments; Rust/TypeScript/JavaScript (and anything else) use `//`.
fn generated_header(path: &str) -> &'static str {
    if path.ends_with(".py") || path.ends_with(".pyi") {
        "# @generated — do not edit by hand.\n"
    } else {
        "// @generated — do not edit by hand.\n"
    }
}

pub fn write_generated_code(generated_code: &GeneratedCode, output_dir: &Path) -> Result<()> {
    fs::create_dir_all(output_dir)?;

    for (relative_path, content) in &generated_code.files {
        let file_path = output_dir.join(relative_path);

        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let header = generated_header(relative_path);
        let final_content = if content.starts_with(header) {
            content.clone()
        } else {
            format!("{header}{content}")
        };

        fs::write(&file_path, final_content)?;
    }

    tracing::info!("Successfully wrote all generated files");
    Ok(())
}

/// Removes the files named one of `names`, in `output_dir` or one level below, that this
/// generation didn't emit, when they are generated files (they start with the generated
/// header). So a regen that stops emitting an optional file (the `authz` modules of a
/// service whose requests are now all skipped, say) doesn't leave a stale one behind.
pub fn remove_stale_generated(
    generated_code: &GeneratedCode,
    output_dir: &Path,
    names: &[&str],
) -> Result<()> {
    let mut dirs = vec![output_dir.to_path_buf()];
    if let Ok(entries) = fs::read_dir(output_dir) {
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                dirs.push(entry.path());
            }
        }
    }
    for dir in dirs {
        for name in names {
            let path = dir.join(name);
            let Ok(relative) = path.strip_prefix(output_dir) else {
                continue;
            };
            let relative = relative.to_string_lossy().replace('\\', "/");
            if generated_code.files.contains_key(&relative) {
                continue;
            }
            let Ok(content) = fs::read_to_string(&path) else {
                continue;
            };
            if content.starts_with(generated_header(&relative)) {
                fs::remove_file(&path)?;
                tracing::info!(path = %path.display(), "removed a stale generated file");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn create_test_generated_code() -> GeneratedCode {
        let mut files = HashMap::new();
        files.insert(
            "test_handler.rs".to_string(),
            "// Test handler content\npub trait TestHandler {}\n".to_string(),
        );
        files.insert(
            "test_routes.rs".to_string(),
            "// Test routes content\npub fn test_route() {}\n".to_string(),
        );

        GeneratedCode { files }
    }

    #[test]
    fn removes_stale_generated_files_only() {
        let dir = TempDir::new().unwrap();
        let svc = dir.path().join("svc");
        fs::create_dir_all(&svc).unwrap();
        fs::write(
            svc.join("authz.rs"),
            "// @generated — do not edit by hand.\nold\n",
        )
        .unwrap();
        fs::write(dir.path().join("authz.rs"), "hand-written\n").unwrap();
        let mut files = HashMap::new();
        files.insert("other/authz.rs".to_string(), "x".to_string());
        let code = GeneratedCode { files };
        remove_stale_generated(&code, dir.path(), &["authz.rs"]).unwrap();
        assert!(
            !svc.join("authz.rs").exists(),
            "stale generated file is removed"
        );
        assert!(
            dir.path().join("authz.rs").exists(),
            "a hand-written file is kept"
        );
    }

    #[test]
    fn test_write_generated_code() {
        let temp_dir = TempDir::new().unwrap();
        let generated_code = create_test_generated_code();

        let result = write_generated_code(&generated_code, temp_dir.path());
        assert!(result.is_ok());

        assert!(temp_dir.path().join("test_handler.rs").exists());
        assert!(temp_dir.path().join("test_routes.rs").exists());
    }
}
