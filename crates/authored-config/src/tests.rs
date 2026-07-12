use super::*;

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct SimpleConfig {
    name: String,
    count: u64,
}

#[test]
fn detect_authored_config_format_classifies_prefixes() {
    for (raw, expected) in [
        ("  {\"k\":1}", AuthoredConfigFormat::Json),
        (" \n [1,2,3]", AuthoredConfigFormat::Json),
        ("title = \"demo\"", AuthoredConfigFormat::Toml),
        ("", AuthoredConfigFormat::Toml),
    ] {
        assert_eq!(detect_authored_config_format(raw), expected, "{raw:?}");
    }
}

#[test]
fn parse_with_hint_uses_fallback_only_without_explicit_extension() {
    let without_extension = parse_authored_config_with_hint(
        "answer = 41",
        None,
        |_| Err("json"),
        |_| Ok::<_, &'static str>(41_u64),
    )
    .expect("fallback parse");
    assert_eq!(without_extension, 41);

    let json_hint = parse_authored_config_with_hint(
        "answer = 41",
        Some(Path::new("config.json")),
        |_| Err("json"),
        |_| Ok::<_, &'static str>(41_u64),
    )
    .expect_err("json hint should disable fallback");
    assert_eq!(json_hint, "json");
}

#[test]
fn rest_config_values_default_to_transport_appropriate_formats() {
    for (value, expected) in [
        (
            serde_json::json!("name = \"demo\"\ncount = 2\n"),
            AuthoredConfigFormat::Toml,
        ),
        (
            serde_json::json!({"name":"demo","count":2}),
            AuthoredConfigFormat::Json,
        ),
    ] {
        let authored = AuthoredConfig::from_json_transport_value(None, &value).expect("authored");

        assert_eq!(authored.format(), expected);
    }
}

#[test]
fn normalize_toml_config_to_canonical_json() {
    let authored = AuthoredConfig::new(AuthoredConfigFormat::Toml, "name = \"demo\"\ncount = 2\n")
        .expect("authored");

    let normalized = authored.normalize::<SimpleConfig>().expect("normalized");

    assert_eq!(
        normalized.value,
        SimpleConfig {
            name: "demo".to_owned(),
            count: 2
        }
    );
    assert_eq!(normalized.format, AuthoredConfigFormat::Toml);
    assert_eq!(
        normalized.canonical_json.as_bytes(),
        br#"{"count":2,"name":"demo"}"#
    );
}

#[test]
fn normalize_rejects_invalid_submissions_with_stable_error_codes() {
    enum Case {
        DuplicateJsonKeys,
        UnknownFields,
        Float,
    }

    for (case, expected_code) in [
        (Case::DuplicateJsonKeys, "AuthoredConfigInvalidJson"),
        (Case::UnknownFields, "AuthoredConfigUnknownField"),
        (Case::Float, "AuthoredConfigFloatUnsupported"),
    ] {
        let err = match case {
            Case::DuplicateJsonKeys => AuthoredConfig::new(
                AuthoredConfigFormat::Json,
                r#"{"name":"a","name":"b","count":1}"#,
            )
            .expect("authored")
            .normalize::<SimpleConfig>()
            .expect_err("duplicate key should reject"),
            Case::UnknownFields => AuthoredConfig::new(
                AuthoredConfigFormat::Json,
                r#"{"name":"demo","count":1,"extra":true}"#,
            )
            .expect("authored")
            .normalize::<SimpleConfig>()
            .expect_err("unknown field should reject"),
            Case::Float => AuthoredConfig::new(AuthoredConfigFormat::Json, r#"{"amount":1.25}"#)
                .expect("authored")
                .normalize::<serde_json::Value>()
                .expect_err("float should reject"),
        };

        assert_eq!(err.code(), expected_code);
    }
}

#[test]
fn authored_config_size_limit_is_enforced_before_parse() {
    let err = AuthoredConfig::with_size_limit(AuthoredConfigFormat::Toml, "name = \"demo\"", 4)
        .expect_err("size limit");

    assert_eq!(err.code(), "AuthoredConfigTooLarge");
}

#[test]
fn normalize_rejects_invalid_toml_without_echoing_input() {
    let authored =
        AuthoredConfig::new(AuthoredConfigFormat::Toml, "name = @secret").expect("authored");

    let err = authored
        .normalize::<SimpleConfig>()
        .expect_err("invalid TOML");

    assert_eq!(err.code(), "AuthoredConfigInvalidToml");
    assert!(!err.message().contains("secret"));
}
