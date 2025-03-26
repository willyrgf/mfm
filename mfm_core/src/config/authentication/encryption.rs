use base64::{engine::general_purpose::STANDARD, Engine};
use ring::{
    aead::{self, BoundKey, OpeningKey, SealingKey, UnboundKey, NONCE_LEN},
    pbkdf2::{self, PBKDF2_HMAC_SHA256},
    rand::SecureRandom,
};
use zeroize::Zeroizing;

const SALT: &[u8] = b"mfm_encryption_salt";
const ITERATIONS: u32 = 100_000;
const KEY_LENGTH: usize = 32; // ChaCha20-Poly1305 uses 32-byte keys

struct NonceSequence {
    nonce: [u8; NONCE_LEN],
}

impl Clone for NonceSequence {
    fn clone(&self) -> Self {
        Self { nonce: self.nonce }
    }
}

impl NonceSequence {
    fn new() -> Self {
        let mut nonce = [0u8; NONCE_LEN];
        ring::rand::SystemRandom::new()
            .fill(&mut nonce)
            .expect("Failed to generate nonce");
        Self { nonce }
    }
}

impl aead::NonceSequence for NonceSequence {
    fn advance(&mut self) -> std::result::Result<aead::Nonce, ring::error::Unspecified> {
        Ok(aead::Nonce::assume_unique_for_key(self.nonce))
    }
}

pub struct Encryption {
    #[allow(dead_code)]
    sealing_key: SealingKey<NonceSequence>,
    opening_key: OpeningKey<NonceSequence>,
    nonce: [u8; NONCE_LEN],
    password: String,
}

impl Encryption {
    pub fn new(password: &str) -> Self {
        let key = Self::derive_key(password);
        let unbound_key1 = UnboundKey::new(&aead::CHACHA20_POLY1305, &key).unwrap();
        let unbound_key2 = UnboundKey::new(&aead::CHACHA20_POLY1305, &key).unwrap();
        let nonce_sequence = NonceSequence::new();
        let nonce = nonce_sequence.nonce;
        let sealing_key = SealingKey::new(unbound_key1, nonce_sequence.clone());
        let opening_key = OpeningKey::new(unbound_key2, nonce_sequence);
        Self {
            sealing_key,
            opening_key,
            nonce,
            password: password.to_string(),
        }
    }

    fn derive_key(password: &str) -> [u8; KEY_LENGTH] {
        let mut key_bytes = [0u8; KEY_LENGTH];
        pbkdf2::derive(
            PBKDF2_HMAC_SHA256,
            std::num::NonZeroU32::new(ITERATIONS).unwrap(),
            SALT,
            password.as_bytes(),
            &mut key_bytes,
        );
        key_bytes
    }

    #[allow(dead_code)]
    pub fn encrypt(&mut self, data: &str) -> Result<String, Box<dyn std::error::Error>> {
        // Validate private key format (64 hex characters)
        if !data.chars().all(|c| c.is_ascii_hexdigit()) || data.len() != 64 {
            return Err("Invalid private key format. Must be 64 hexadecimal characters.".into());
        }

        let nonce_sequence = NonceSequence { nonce: self.nonce };
        self.sealing_key = SealingKey::new(
            UnboundKey::new(&aead::CHACHA20_POLY1305, &Self::derive_key(&self.password)).unwrap(),
            nonce_sequence,
        );

        let mut in_out = data.as_bytes().to_vec();
        let tag = self
            .sealing_key
            .seal_in_place_separate_tag(aead::Aad::empty(), &mut in_out)
            .map_err(|e| format!("Encryption failed: {}", e))?;

        let mut result = Vec::with_capacity(NONCE_LEN + in_out.len() + tag.as_ref().len());
        result.extend_from_slice(&self.nonce);
        result.extend_from_slice(&in_out);
        result.extend_from_slice(tag.as_ref());

        Ok(STANDARD.encode(&result))
    }

    pub fn decrypt(
        &mut self,
        encrypted_data: &str,
    ) -> Result<Zeroizing<String>, Box<dyn std::error::Error>> {
        let data = STANDARD
            .decode(encrypted_data)
            .map_err(|e| format!("Base64 decode failed: {}", e))?;

        if data.len() < NONCE_LEN {
            return Err("Invalid encrypted data length".into());
        }

        let (nonce_bytes, rest) = data.split_at(NONCE_LEN);
        self.nonce = nonce_bytes.try_into().unwrap();
        let nonce_sequence = NonceSequence { nonce: self.nonce };
        self.opening_key = OpeningKey::new(
            UnboundKey::new(&aead::CHACHA20_POLY1305, &Self::derive_key(&self.password)).unwrap(),
            nonce_sequence,
        );

        let mut in_out = rest.to_vec();
        let decrypted = self
            .opening_key
            .open_in_place(aead::Aad::empty(), &mut in_out)
            .map_err(|e| format!("Decryption failed: {}", e))?;

        let result =
            String::from_utf8(decrypted.to_vec()).map_err(|e| format!("Invalid UTF-8: {}", e))?;

        Ok(Zeroizing::new(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encryption_decryption() {
        let password = "test_password";
        let private_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

        let mut encryption = Encryption::new(password);
        let encrypted = encryption.encrypt(private_key).unwrap();
        let decrypted = encryption.decrypt(&encrypted).unwrap();

        assert_eq!(decrypted.as_str(), private_key);
    }
}
