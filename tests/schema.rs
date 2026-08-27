use lpt_lib::schema::*;

#[test]
fn schema_is_valid_json_and_has_required_fields() {
    let s = generate_schema();
    assert_eq!(s["required"][0], "package_name");
    assert_eq!(s["required"][1], "github_repo");
    assert!(s["properties"]["suggests"].is_object());
    assert!(s["properties"]["predepends"].is_object());
    assert!(s["properties"]["section"].is_object());
    assert!(s["properties"]["priority"].is_object());
    assert!(s["properties"]["fields"].is_object());
    assert!(s["properties"]["compression"].is_object());
}

#[test]
fn schema_compression_enum_covers_all() {
    let s = generate_schema();
    let enums = s["properties"]["compression"]["enum"].as_array().unwrap();
    let vals: Vec<&str> = enums.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(vals.contains(&"gzip"));
    assert!(vals.contains(&"xz"));
    assert!(vals.contains(&"zstd"));
    assert!(vals.contains(&"none"));
}

#[test]
fn schema_documents_new_features() {
    let s = generate_schema();
    for key in [
        "contents",
        "overrides",
        "scripts",
        "signature",
        "local_payload",
    ] {
        assert!(s["properties"][key].is_object(), "missing {key}");
    }
    let content_types = s["properties"]["contents"]["items"]["properties"]["type"]["enum"]
        .as_array()
        .unwrap();
    let vals: Vec<&str> = content_types.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(vals.contains(&"config|noreplace"));
    assert!(vals.contains(&"tree"));
    assert!(vals.contains(&"symlink"));
    assert!(vals.contains(&"ghost"));
    let methods = s["properties"]["signature"]["properties"]["method"]["enum"]
        .as_array()
        .unwrap();
    let methods: Vec<&str> = methods.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(methods.contains(&"detach"));
    assert!(methods.contains(&"debsign"));
}
