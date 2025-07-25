use mfm::cli::utils::output::{format_keys, KeyDisplay};
use mfm::cli::OutputFormat;

#[test]
fn test_output_format_values() {
    // Test the OutputFormat enum variants
    assert!(matches!(OutputFormat::Text, OutputFormat::Text));
    assert!(matches!(OutputFormat::Json, OutputFormat::Json));
}

#[test]
fn test_key_display_table_formatting() {
    let keys = vec![
        KeyDisplay {
            id: "test-id-1".to_string(),
            label: "test-key-1".to_string(),
            key_type: "privatekey".to_string(),
            address: None,
            created: "2024-01-01 12:00:00".to_string(),
        },
        KeyDisplay {
            id: "test-id-2".to_string(),
            label: "test-key-2".to_string(),
            key_type: "mnemonic".to_string(),
            address: None,
            created: "2024-01-01 13:00:00".to_string(),
        },
    ];

    let output = format_keys(keys, OutputFormat::Text, false);

    // Verify table contains expected data
    assert!(output.contains("test-id-1"));
    assert!(output.contains("test-key-1"));
    assert!(output.contains("privatekey"));
    assert!(output.contains("test-id-2"));
    assert!(output.contains("test-key-2"));
    assert!(output.contains("mnemonic"));
}

#[test]
fn test_key_display_json_formatting() {
    let keys = vec![KeyDisplay {
        id: "test-id-1".to_string(),
        label: "test-key-1".to_string(),
        key_type: "privatekey".to_string(),
        address: Some("0x1234567890123456789012345678901234567890".to_string()),
        created: "2024-01-01 12:00:00".to_string(),
    }];

    let output = format_keys(keys, OutputFormat::Json, true);

    // Verify JSON contains expected data
    assert!(output.contains("test-id-1"));
    assert!(output.contains("test-key-1"));
    assert!(output.contains("privatekey"));
    assert!(output.contains("0x1234567890123456789012345678901234567890"));
    assert!(output.contains("2024-01-01 12:00:00"));

    // Verify it's valid JSON with success response structure
    let parsed: serde_json::Value = serde_json::from_str(&output).expect("Should be valid JSON");
    assert_eq!(parsed["status"], "success");
    assert!(parsed["data"].is_array());
}

#[test]
fn test_key_display_json_without_addresses() {
    let keys = vec![KeyDisplay {
        id: "test-id-1".to_string(),
        label: "test-key-1".to_string(),
        key_type: "privatekey".to_string(),
        address: Some("0x1234567890123456789012345678901234567890".to_string()),
        created: "2024-01-01 12:00:00".to_string(),
    }];

    let output = format_keys(keys, OutputFormat::Json, false);

    // Should not contain address when show_addresses is false
    assert!(!output.contains("0x1234567890123456789012345678901234567890"));

    // Verify it's valid JSON with success response structure
    let parsed: serde_json::Value = serde_json::from_str(&output).expect("Should be valid JSON");
    assert_eq!(parsed["status"], "success");
    assert!(parsed["data"].is_array());

    // Verify address field is null
    let first_key = &parsed["data"][0];
    assert!(first_key["address"].is_null());
}

#[test]
fn test_key_display_table_with_addresses() {
    let keys = vec![KeyDisplay {
        id: "test-id-1".to_string(),
        label: "test-key-1".to_string(),
        key_type: "privatekey".to_string(),
        address: Some("0x1234567890123456789012345678901234567890".to_string()),
        created: "2024-01-01 12:00:00".to_string(),
    }];

    let output = format_keys(keys, OutputFormat::Text, true);

    // Should contain address when show_addresses is true
    assert!(output.contains("0x1234567890123456789012345678901234567890"));
}

#[test]
fn test_key_display_table_without_addresses() {
    let keys = vec![KeyDisplay {
        id: "test-id-1".to_string(),
        label: "test-key-1".to_string(),
        key_type: "privatekey".to_string(),
        address: Some("0x1234567890123456789012345678901234567890".to_string()),
        created: "2024-01-01 12:00:00".to_string(),
    }];

    let output = format_keys(keys, OutputFormat::Text, false);

    // Should not contain address when show_addresses is false
    assert!(!output.contains("0x1234567890123456789012345678901234567890"));
}

#[test]
fn test_empty_keys_formatting() {
    let keys = vec![];

    let table_output = format_keys(keys.clone(), OutputFormat::Text, false);
    let json_output = format_keys(keys, OutputFormat::Json, false);

    // Table should be empty or just headers
    assert!(!table_output.contains("test-id"));

    // JSON should be success response with empty array
    let parsed: serde_json::Value =
        serde_json::from_str(&json_output).expect("Should be valid JSON");
    assert_eq!(parsed["status"], "success");
    assert!(parsed["data"].is_array());
    assert_eq!(parsed["data"].as_array().unwrap().len(), 0);
}
