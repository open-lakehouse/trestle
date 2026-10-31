//! Generates the buffa models, the trestle client and models `mod.rs`, and the model rewrites
//! into `OUT_DIR`. The `py` crate generates the Python bindings.

include!("config.rs");

use olai_codegen::{generate_code, parse_file_descriptor_set};
use protobuf::Message;
use protobuf::descriptor::FileDescriptorSet;

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let descriptors = compile_descriptors(&out);
    let config = demo_config(&out);
    let gen_dir = config.output.models_subdir_path().unwrap();

    // buffa-build writes `<package>.mod.rs`; the models `mod.rs` includes the buf plugin's
    // `<package>.rs`, so bridge the two.
    buffa_build::Config::new()
        .files(&[fixture_dir().join("proto/demo/v1/svc.proto")])
        .includes(&[fixture_dir().join("proto")])
        .generate_json(true)
        .generate_views(true)
        .out_dir(&gen_dir)
        .include_file("_include.rs")
        .compile()
        .unwrap();
    std::fs::write(
        gen_dir.join("demo.v1.rs"),
        "include!(\"demo.v1.mod.rs\");\n",
    )
    .unwrap();

    let fds = FileDescriptorSet::parse_from_bytes(&std::fs::read(&descriptors).unwrap()).unwrap();
    let metadata = parse_file_descriptor_set(&fds).unwrap();
    generate_code(&metadata, &config).unwrap();
    demo_rewrites(&config, &metadata)
        .rewrite_dir(&gen_dir)
        .unwrap();

    // `#[path]` needs a literal path, so mount the generated `mod.rs` through a small file.
    std::fs::write(
        out.join("models_mod.rs"),
        format!(
            "#[path = {:?}]\npub mod _gen;\n",
            gen_dir.join("mod.rs").display().to_string()
        ),
    )
    .unwrap();
}
