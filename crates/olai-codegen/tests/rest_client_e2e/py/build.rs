//! Generates the Python bindings and package files into `OUT_DIR`, from the client crate's config.

include!("../config.rs");

use olai_codegen::{generate_code, parse_file_descriptor_set};
use protobuf::Message;
use protobuf::descriptor::FileDescriptorSet;

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let descriptors = compile_descriptors(&out);
    let config = demo_config(&out);
    let fds = FileDescriptorSet::parse_from_bytes(&std::fs::read(&descriptors).unwrap()).unwrap();
    let metadata = parse_file_descriptor_set(&fds).unwrap();
    generate_code(&metadata, &config).unwrap();
    std::fs::write(out.join("pkg/demo/__init__.py"), "").unwrap();

    // `#[path]` needs a literal path, so mount the generated `mod.rs` through a small file.
    std::fs::write(
        out.join("python_mod.rs"),
        format!(
            "#[path = {:?}]\npub mod codegen;\n",
            out.join("python/mod.rs").display().to_string()
        ),
    )
    .unwrap();
}
