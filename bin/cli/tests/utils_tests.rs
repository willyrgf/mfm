use mfm::presentation::output::{format_keys_table, KeyDisplay, ResponseStatus, SuccessResponse};

fn key_display(
    id: &str,
    label: &str,
    key_type: &str,
    address: Option<&str>,
    created: &str,
) -> KeyDisplay {
    KeyDisplay {
        id: id.to_string(),
        label: label.to_string(),
        key_type: key_type.to_string(),
        address: address.map(str::to_string),
        created: created.to_string(),
    }
}

#[test]
fn test_key_display_table_formatting() {
    let keys = vec![
        key_display(
            "test-id-1",
            "test-key-1",
            "privatekey",
            None,
            "2024-01-01 12:00:00",
        ),
        key_display(
            "test-id-2",
            "test-key-2",
            "hd_derived",
            None,
            "2024-01-01 13:00:00",
        ),
    ];

    let output = format_keys_table(&keys, false);

    // Verify table contains expected data
    assert!(output.contains("test-id-1"));
    assert!(output.contains("test-key-1"));
    assert!(output.contains("privatekey"));
    assert!(output.contains("test-id-2"));
    assert!(output.contains("test-key-2"));
    assert!(output.contains("hd_derived"));
}

#[test]
fn test_key_display_json_formatting() {
    let keys = vec![key_display(
        "test-id-1",
        "test-key-1",
        "privatekey",
        Some("0x1234567890123456789012345678901234567890"),
        "2024-01-01 12:00:00",
    )];

    let response = SuccessResponse::new(keys.clone());
    let output = serde_json::to_string(&response).unwrap();

    // Verify JSON contains expected data
    assert!(output.contains("test-id-1"));
    assert!(output.contains("test-key-1"));
    assert!(output.contains("privatekey"));
    assert!(output.contains("0x1234567890123456789012345678901234567890"));
    assert!(output.contains("2024-01-01 12:00:00"));

    // Verify it's valid JSON with success response structure
    let parsed: SuccessResponse<Vec<KeyDisplay>> =
        serde_json::from_str(&output).expect("Should be valid JSON");
    assert!(matches!(parsed.status, ResponseStatus::Success));
    assert_eq!(parsed.data.len(), 1);
}

#[test]
fn test_key_display_json_without_addresses() {
    let keys = vec![key_display(
        "test-id-1",
        "test-key-1",
        "privatekey",
        Some("0x1234567890123456789012345678901234567890"),
        "2024-01-01 12:00:00",
    )];

    // In the new model, filtering addresses is done before serialization.
    let keys_without_address: Vec<KeyDisplay> = keys
        .into_iter()
        .map(|mut k| {
            k.address = None;
            k
        })
        .collect();
    let response = SuccessResponse::new(keys_without_address);
    let output = serde_json::to_string(&response).unwrap();

    // Should not contain address when show_addresses is false
    assert!(!output.contains("0x1234567890123456789012345678901234567890"));

    // Verify it's valid JSON with success response structure
    let parsed: SuccessResponse<Vec<KeyDisplay>> =
        serde_json::from_str(&output).expect("Should be valid JSON");
    assert!(matches!(parsed.status, ResponseStatus::Success));

    // Verify address field is null
    let first_key = &parsed.data[0];
    assert!(first_key.address.is_none());
}

#[test]
fn test_key_display_table_address_visibility() {
    const ADDRESS: &str = "0x1234567890123456789012345678901234567890";
    let keys = vec![key_display(
        "test-id-1",
        "test-key-1",
        "privatekey",
        Some(ADDRESS),
        "2024-01-01 12:00:00",
    )];

    for (show_addresses, should_contain) in [(true, true), (false, false)] {
        let output = format_keys_table(&keys, show_addresses);
        assert_eq!(
            output.contains(ADDRESS),
            should_contain,
            "show_addresses={show_addresses}"
        );
    }
}

#[test]
fn test_empty_keys_table_and_json_output() {
    let keys: Vec<KeyDisplay> = Vec::new();
    let table_output = format_keys_table(&keys, false);
    let lines: Vec<&str> = table_output.lines().collect();

    assert_eq!(lines.len(), 3);
    assert!(lines[0].starts_with('+') && lines[0].ends_with('+'));
    assert!(lines[1].contains("| id"));
    assert_eq!(lines[0], lines[2]);

    let response = SuccessResponse::new(keys);
    let json_output = serde_json::to_string(&response).unwrap();
    let parsed: SuccessResponse<Vec<KeyDisplay>> =
        serde_json::from_str(&json_output).expect("Should be valid JSON");

    assert!(matches!(parsed.status, ResponseStatus::Success));
    assert!(parsed.data.is_empty());
}

#[test]
fn test_ascii_table_shape_is_stable() {
    let keys = vec![
        key_display("id-1", "short", "privatekey", None, "2024-01-01 12:00:00"),
        key_display(
            "id-2",
            "much-longer-label",
            "hd_derived",
            None,
            "2024-01-01 13:00:00",
        ),
    ];

    let output = format_keys_table(&keys, false);
    let lines: Vec<&str> = output.lines().collect();

    assert!(lines.len() >= 5);
    assert!(lines[0].starts_with('+') && lines[0].ends_with('+'));
    assert_eq!(lines[0], lines[2]);
    assert!(lines[1].contains("| id"));
    assert!(lines[1].contains("| label"));
    assert!(lines[1].contains("| key_type"));
    assert!(lines[1].contains("| created"));

    for line in &lines {
        if line.starts_with('|') {
            assert_eq!(line.len(), lines[0].len());
        }
    }
}
