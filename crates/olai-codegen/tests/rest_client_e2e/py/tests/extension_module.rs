//! The bindings built as an extension module: a Python process imports it as `demo._core`, and
//! `pyo3-introspection` writes its stubs.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The cdylib of this crate, which cargo builds for the tests: next to the test binary
/// (`target/debug/deps`), or with the build-dir layout in a sibling `<hash>/out` directory of
/// the package. The newest one when there are several.
fn cdylib() -> PathBuf {
    let name = format!(
        "{}rest_client_e2e_py{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let exe = std::env::current_exe().unwrap();
    let mut found = Vec::new();
    for dir in exe.ancestors().skip(1).take(3) {
        find(dir, &name, 2, &mut found);
    }
    found
        .into_iter()
        .max_by_key(|path| std::fs::metadata(path).and_then(|m| m.modified()).ok())
        .unwrap_or_else(|| panic!("{name} is not built near {}", exe.display()))
}

/// The files named `name` in `dir` and its subdirectories, `depth` levels down.
fn find(dir: &Path, name: &str, depth: usize, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth > 0 {
                find(&path, name, depth - 1, found);
            }
        } else if entry.file_name() == name {
            found.push(path);
        }
    }
}

/// Copy the directory `from` into `to`.
fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// The import system runs the module's init, which puts the submodules in `sys.modules`.
#[test]
fn imports_as_a_package_module() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("extension_module");
    let _ = std::fs::remove_dir_all(&dir);
    copy_dir(Path::new(rest_client_e2e_py::PACKAGE_DIR), &dir);
    std::fs::copy(cdylib(), dir.join("demo/_core.so")).unwrap();

    let python = std::env::var("PYO3_PYTHON").unwrap_or_else(|_| "python3".into());
    let output = Command::new(python)
        .arg("-c")
        .arg(
            "import demo._core.demo\n\
             from demo._core import DemoClient\n\
             from demo._core.demo import Widget\n\
             assert DemoClient.__module__ == 'demo._core', DemoClient.__module__\n\
             assert Widget.__module__ == 'demo._core.demo', Widget.__module__\n\
             assert demo._core.__version__ == '0.0.0', demo._core.__version__\n",
        )
        .env("PYTHONPATH", &dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The stubs that PyO3's introspection writes from the extension module.
#[test]
fn stubs() {
    let module = pyo3_introspection::introspect_cdylib(cdylib(), "_core").unwrap();
    let files = pyo3_introspection::module_stub_files(&module);
    let root = &files[Path::new("__init__.pyi")];
    let models = &files[Path::new("demo.pyi")];
    for expected in [
        "\"\"\"\nThe extension module `demo._core`.\n\"\"\"",
        "from . import demo as demo",
        "__version__: Final[str]",
        "class DemoClient:",
        "def list_widgets(self, /, *, max_results: SupportsIndex | None = None, \
         color: Color | str | None = None, first: SupportsIndex | None = None, \
         order_by: str | None = None) -> list[Widget]:",
        "-> Future[list[Widget]]:",
        "def widget(self, /, widget_name: str) -> WidgetClient:",
        "class DemoError(Exception):",
        "class NotFoundError(DemoError):",
        "def details(self, /) -> list:",
        "def get_blob(self, /, path: str) -> Blob:",
        "            order_by: The order of the widgets.",
    ] {
        assert!(root.contains(expected), "missing `{expected}` in:\n{root}");
    }
    // Not handed the module, the init leaves the stubs complete.
    assert!(!root.contains("__getattr__"), "{root}");
    for expected in [
        "from demo._enums import Color",
        "class Widget:",
        "def color(self, /) -> Color | None:",
        "def type(self, /) -> str | None:",
        "def __eq__(self, value: object, /) -> bool: ...",
        "The color of the widget.",
        "def __new__(cls, /, key: str, *, value: str | None = None",
        "def get_async(self, /) -> Future[Widget]:",
        "def delete_async(self, /) -> Future[None]:",
        "def attributes(self, /) -> dict[str, Any] | None:",
        "def extra(self, /) -> Any:",
        "def notes(self, /) -> list[Any]:",
        "def rows(self, /) -> dict[str, list[Any]]:",
    ] {
        assert!(
            models.contains(expected),
            "missing `{expected}` in:\n{models}"
        );
    }
    // No request classes; no constructor on response models.
    assert!(!models.contains("class CreateWidgetRequest"), "{models}");
    let widget = &models[models.find("class Widget:").unwrap()..];
    let widget = &widget[..widget.find("class WidgetClient:").unwrap()];
    assert!(!widget.contains("__new__"), "{widget}");
}
