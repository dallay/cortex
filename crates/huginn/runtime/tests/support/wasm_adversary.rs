use std::path::PathBuf;

pub fn fixture(body: &str, output: &str) -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let literal: String = output.bytes().map(|byte| format!("\\{byte:02x}")).collect();
    let module = format!(
        r#"(module
      (import "huginn:rust-outline/snapshots@0.1.0" "[method]snapshot.size" (func $size (param i32) (result i32)))
      (import "huginn:rust-outline/snapshots@0.1.0" "[method]snapshot.read" (func $read (param i32 i32 i32 i32)))
      (import "huginn:rust-outline/snapshots@0.1.0" "[resource-drop]snapshot" (func $drop (param i32)))
      (memory (export "memory") 2)
      (func (export "cabi_realloc") (param i32 i32 i32 i32) (result i32) i32.const 4096)
      (data (i32.const 32) "{literal}")
      (func (export "outline") (param $file i32) (result i32)
        local.get $file call $size drop
        {body}
        local.get $file call $drop
        i32.const 0 i32.const 32 i32.store
        i32.const 4 i32.const {length} i32.store
        i32.const 0))"#,
        length = output.len()
    );
    let mut module = wat::parse_str(module).unwrap();
    let mut resolve = wit_parser::Resolve::default();
    let wit = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../plugin-api/wit/experimental-rust-outline");
    let (package, _) = resolve.push_dir(wit).unwrap();
    let world = resolve
        .select_world(&[package], Some("rust-outline"))
        .unwrap();
    wit_component::embed_component_metadata(
        &mut module,
        &resolve,
        world,
        wit_component::StringEncoding::UTF8,
        false,
    )
    .unwrap();
    let bytes = wit_component::ComponentEncoder::default()
        .module(&module)
        .unwrap()
        .validate(true)
        .encode()
        .unwrap();
    let path = directory.path().join("adversary.component.wasm");
    std::fs::write(&path, bytes).unwrap();
    (directory, path)
}
