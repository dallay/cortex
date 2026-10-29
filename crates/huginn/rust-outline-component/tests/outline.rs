use huginn_rust_outline_component::outline;

#[test]
fn declarations_include_original_signatures_and_half_open_byte_ranges() {
    let source = "// heading\npub fn greet(name: &str) -> usize { name.len() }\n";
    let result = outline(source);
    assert_eq!(result["complete"], true);
    let entry = &result["declarations"][0];
    assert_eq!(entry["name"], "greet");
    assert_eq!(entry["kind"], "function");
    assert_eq!(entry["signature"], "pub fn greet(name: &str) -> usize");
    let start = entry["range"]["start_byte"].as_u64().unwrap() as usize;
    let end = entry["range"]["end_byte"].as_u64().unwrap() as usize;
    assert_eq!(
        &source[start..end],
        "pub fn greet(name: &str) -> usize { name.len() }"
    );
    assert_eq!(entry["range"]["start_line"], 2);
    assert_eq!(entry["range"]["start_column"], 1);
}

#[test]
fn containers_include_associated_types_and_constants_without_following_modules() {
    let source = "mod external;\nmod inner { pub trait Store { type Key; const LIMIT: usize; fn get(&self); } impl Store for Cache { type Key = u64; const LIMIT: usize = 8; fn get(&self) {} } }";
    let result = outline(source);
    let entries = result["declarations"].as_array().unwrap();
    assert!(entries.iter().any(|entry| entry["name"] == "Key"
        && entry["kind"] == "type"
        && entry["container"] == "inner::Store"));
    assert!(entries.iter().any(|entry| entry["name"] == "LIMIT"
        && entry["kind"] == "const"
        && entry["container"] == "inner::Cache"));
    assert!(entries
        .iter()
        .any(|entry| entry["name"] == "get" && entry["signature"] == "fn get(&self)"));
}

#[test]
fn unicode_crlf_and_shebang_preserve_exact_source_ranges() {
    let source = "#!/usr/bin/env rust-script\r\n// café\r\npub fn café() { let _ = \"你好\"; }\r\n";
    let result = outline(source);
    let entry = &result["declarations"][0];
    assert_eq!(entry["name"], "café");
    let start = entry["range"]["start_byte"].as_u64().unwrap() as usize;
    let end = entry["range"]["end_byte"].as_u64().unwrap() as usize;
    assert_eq!(&source[start..end], "pub fn café() { let _ = \"你好\"; }");
    assert_eq!(entry["range"]["start_line"], 3);
    assert_eq!(entry["range"]["start_column"], 1);
}

#[test]
fn deferred_positions_preserve_nested_unicode_crlf_bom_shebang_and_eof_ranges() {
    for source in [
        "\u{feff}#!/usr/bin/env rust-script\r\nmod café {\r\n /*你好*/ pub fn élève() {}\r\n}",
        "\u{feff}mod café { /*你好*/ pub fn élève() {} }",
        "mod outer { mod inner { fn café() {} } fn last() {} }",
        "mod outer { fn last() {}}",
    ] {
        let result = outline(source);
        assert_eq!(result["complete"], true, "{result}");
        let entries = result["declarations"].as_array().unwrap();
        assert!(entries.len() >= 2);
        assert_eq!(entries[0]["range"]["end_byte"], source.len());
        for entry in entries {
            for endpoint in ["start", "end"] {
                let offset = entry["range"][format!("{endpoint}_byte")].as_u64().unwrap() as usize;
                let prefix = &source[..offset];
                let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
                let column = prefix.rsplit('\n').next().unwrap().chars().count() + 1;
                assert_eq!(entry["range"][format!("{endpoint}_line")], line);
                assert_eq!(entry["range"][format!("{endpoint}_column")], column);
            }
        }
    }
}

#[test]
fn syntax_errors_and_size_limits_are_explicit_not_empty_success() {
    for source in ["pub fn broken(", &" ".repeat(1_048_577)] {
        let result = outline(source);
        assert_eq!(result["complete"], false);
        assert!(result["declarations"].as_array().unwrap().is_empty());
        assert!(!result["diagnostics"].as_array().unwrap().is_empty());
    }
}

#[test]
fn output_limit_retains_whole_records_and_valid_json() {
    let source: String = (0..2_000)
        .map(|index| format!("pub fn function_{index}() {{}}\n"))
        .collect();
    let result = outline(&source);
    assert_eq!(result["complete"], false);
    assert!(!result["declarations"].as_array().unwrap().is_empty());
    assert!(result.to_string().len() <= 49_152);
}
