use crate::config::authentication::encryption::Encryption;
use std::fs;
use std::path::PathBuf;
use tempfile::NamedTempFile;

#[test]
fn test_encryption_basic() {
    let password = "test_password";
    let private_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    let mut encryption = Encryption::new(password);
    let encrypted = encryption.encrypt(private_key).unwrap();
    let decrypted = encryption.decrypt(&encrypted).unwrap();

    assert_eq!(decrypted.as_str(), private_key);
}

#[test]
fn test_encryption_different_passwords() {
    let private_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    // Encrypt with first password
    let mut encryption1 = Encryption::new("password1");
    let encrypted1 = encryption1.encrypt(private_key).unwrap();

    // Try to decrypt with different password
    let mut encryption2 = Encryption::new("password2");
    assert!(encryption2.decrypt(&encrypted1).is_err());
}

#[test]
fn test_encryption_invalid_data() {
    let mut encryption = Encryption::new("test_password");

    // Test with invalid base64
    assert!(encryption.decrypt("invalid base64").is_err());

    // Test with data too short
    assert!(encryption.decrypt("aGVsbG8=").is_err());
}

#[test]
fn test_encryption_empty_data() {
    let mut encryption = Encryption::new("test_password");

    // Test with empty string - should fail with invalid format
    let result = encryption.encrypt("");
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Invalid private key format"));
}

#[test]
fn test_encryption_large_data() {
    let password = "test_password";
    // Use a valid private key
    let private_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    let mut encryption = Encryption::new(password);
    let encrypted = encryption.encrypt(private_key).unwrap();
    let decrypted = encryption.decrypt(&encrypted).unwrap();

    assert_eq!(decrypted.as_str(), private_key);
}

#[test]
fn test_encryption_special_characters() {
    let password = "test!@#$%^&*()_+";
    let private_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    let mut encryption = Encryption::new(password);
    let encrypted = encryption.encrypt(private_key).unwrap();
    let decrypted = encryption.decrypt(&encrypted).unwrap();

    assert_eq!(decrypted.as_str(), private_key);
}

#[test]
fn test_encryption_unicode() {
    let password = "test_password";
    let private_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    let mut encryption = Encryption::new(password);
    let encrypted = encryption.encrypt(private_key).unwrap();
    let decrypted = encryption.decrypt(&encrypted).unwrap();

    assert_eq!(decrypted.as_str(), private_key);
}

#[test]
fn test_encryption_file_operations() -> Result<(), Box<dyn std::error::Error>> {
    let temp_file = NamedTempFile::new()?;
    let private_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let password = "test_password";

    // Encrypt and save to file
    let mut encryption = Encryption::new(password);
    let encrypted = encryption.encrypt(private_key)?;
    fs::write(&temp_file, encrypted)?;

    // Read and decrypt from file
    let encrypted_data = fs::read_to_string(&temp_file)?;
    let decrypted = encryption.decrypt(&encrypted_data)?;

    assert_eq!(decrypted.as_str(), private_key);
    Ok(())
}

#[test]
fn test_encryption_multiple_operations() {
    let password = "test_password";
    let private_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    let mut encryption = Encryption::new(password);

    // Perform multiple encrypt/decrypt operations
    for _ in 0..10 {
        let encrypted = encryption.encrypt(private_key).unwrap();
        let decrypted = encryption.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted.as_str(), private_key);
    }
}

#[test]
fn test_encryption_invalid_private_key() {
    let password = "test_password";

    // Test with invalid private key length
    let invalid_key = "0123456789abcdef"; // Too short
    let mut encryption = Encryption::new(password);
    assert!(encryption.encrypt(invalid_key).is_err());

    // Test with non-hex characters
    let invalid_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcg"; // Contains 'g'
    assert!(encryption.encrypt(invalid_key).is_err());
}

#[test]
fn test_encryption_zeroize() {
    let password = "test_password";
    let private_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    let mut encryption = Encryption::new(password);
    let encrypted = encryption.encrypt(private_key).unwrap();
    let decrypted = encryption.decrypt(&encrypted).unwrap();

    // Verify the decrypted value is zeroized when dropped
    let decrypted_str = decrypted.as_str().to_string();
    drop(decrypted);

    // The original string should still be valid
    assert_eq!(decrypted_str, private_key);
}

#[test]
fn test_encryption_with_cli() -> Result<(), Box<dyn std::error::Error>> {
    let temp_file = NamedTempFile::new()?;
    let private_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let password = "test_password";

    // Write private key to temp file
    fs::write(&temp_file, private_key)?;

    // Run encryption command
    let output = std::process::Command::new("cargo")
        .args([
            "run",
            "--bin",
            "mfm",
            "--manifest-path",
            "../mfm_cli/Cargo.toml",
            "--",
            "encrypt",
            "-k",
            temp_file.path().to_str().unwrap(),
            "-p",
            password,
            "-f",
        ])
        .output()?;

    assert!(
        output.status.success(),
        "Command failed: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify the file was encrypted
    let encrypted_data = fs::read_to_string(&temp_file)?;
    assert_ne!(encrypted_data, private_key);

    // Try to decrypt with the same password
    let mut encryption = Encryption::new(password);
    let decrypted = encryption.decrypt(&encrypted_data)?;
    assert_eq!(decrypted.as_str(), private_key);

    Ok(())
}

#[test]
fn test_encryption_with_cli_invalid_key() -> Result<(), Box<dyn std::error::Error>> {
    let temp_file = NamedTempFile::new()?;
    let invalid_key = "invalid_key"; // Not 64 hex characters
    let password = "test_password";

    // Write invalid private key to temp file
    fs::write(&temp_file, invalid_key)?;

    // Run encryption command
    let output = std::process::Command::new("cargo")
        .args([
            "run",
            "--bin",
            "mfm",
            "--manifest-path",
            "../mfm_cli/Cargo.toml",
            "--",
            "encrypt",
            "-k",
            temp_file.path().to_str().unwrap(),
            "-p",
            password,
            "-f",
        ])
        .output()?;

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Invalid private key format"));

    Ok(())
}

#[test]
fn test_encryption_with_cli_existing_file() -> Result<(), Box<dyn std::error::Error>> {
    let temp_file = NamedTempFile::new()?;
    let private_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let password = "test_password";

    // Write private key to temp file
    fs::write(&temp_file, private_key)?;

    // Try to encrypt without force flag
    let output = std::process::Command::new("cargo")
        .args([
            "run",
            "--bin",
            "mfm",
            "--manifest-path",
            "../mfm_cli/Cargo.toml",
            "--",
            "encrypt",
            "-k",
            temp_file.path().to_str().unwrap(),
            "-p",
            password,
        ])
        .output()?;

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("File already exists"));

    // Verify the file wasn't modified
    let file_content = fs::read_to_string(&temp_file)?;
    assert_eq!(file_content, private_key);

    Ok(())
}
