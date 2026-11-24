//! `src/gen/` is generated from `proto/`. This test regenerates it and fails when the checked-in
//! copy differs. Run it with `BLESS=1` to write the new copy after changing the proto.

use std::path::{Path, PathBuf};

const GENERATED: &str = "olai.keys.v1.rs";

/// Messages that carry a plaintext DEK. Their `Debug` is written by hand, in `src/lib.rs`.
const SECRET_BEARING: &[&str] = &[".olai.keys.v1.WrapRequest", ".olai.keys.v1.UnwrapResponse"];

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn generate(out_dir: &Path) {
    let root = crate_dir().join("proto");
    let fds = protox::compile([root.join("olai/keys/v1/keys.proto")], [&root]).unwrap();
    tonic_prost_build::configure()
        .build_transport(false)
        .skip_debug(SECRET_BEARING)
        .emit_rerun_if_changed(false)
        .out_dir(out_dir)
        .compile_fds(fds)
        .unwrap();
}

#[test]
fn generated_code_is_current() {
    let out = std::env::temp_dir().join(format!("olai-key-service-gen-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    generate(&out);
    let fresh = std::fs::read_to_string(out.join(GENERATED)).unwrap();
    let _ = std::fs::remove_dir_all(&out);

    let checked_in = crate_dir().join("src/gen").join(GENERATED);
    if std::env::var_os("BLESS").is_some() {
        std::fs::create_dir_all(checked_in.parent().unwrap()).unwrap();
        std::fs::write(&checked_in, fresh).unwrap();
        return;
    }
    let current = std::fs::read_to_string(&checked_in).unwrap_or_default();
    assert!(
        current == fresh,
        "{} is out of date with proto/; rerun with BLESS=1",
        checked_in.display()
    );
}
