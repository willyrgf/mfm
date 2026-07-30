use std::str::FromStr;

use mfm_canonical::{sha256_digest_bytes, ValidatedCanonicalValue};
use mfm_ids::{
    AttemptId, ContentDigest, ContentRef, EffectKey, RequestDigest, SchemaId, TenantScopeId,
};

use crate::contract::{contract_error, recoverability_contract, SchemaQualifiedCanonicalValue};
use crate::{ExecutorError, Result};

pub(crate) const MAX_DURABLE_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;
const MAX_STRING_BYTES: usize = 4096;
const CHECKSUM_BYTES: usize = 32;

pub(crate) struct Encoder {
    bytes: Vec<u8>,
}

impl Encoder {
    pub(crate) fn new(magic: &[u8; 8]) -> Self {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(magic);
        Self { bytes }
    }

    pub(crate) fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub(crate) fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    pub(crate) fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    pub(crate) fn usize(&mut self, value: usize) -> Result<()> {
        self.u64(u64::try_from(value).map_err(|_| ExecutorError::InvalidDurableSnapshot)?);
        Ok(())
    }

    pub(crate) fn string(&mut self, value: &str) -> Result<()> {
        if value.len() > MAX_STRING_BYTES {
            return Err(ExecutorError::InvalidDurableSnapshot);
        }
        self.bytes(value.as_bytes())
    }

    pub(crate) fn bytes(&mut self, value: &[u8]) -> Result<()> {
        if value.len() > MAX_DURABLE_SNAPSHOT_BYTES {
            return Err(ExecutorError::InvalidDurableSnapshot);
        }
        self.usize(value.len())?;
        self.bytes.extend_from_slice(value);
        Ok(())
    }

    pub(crate) fn content_ref(&mut self, value: &ContentRef) -> Result<()> {
        self.string(value.schema_id().as_str())?;
        self.string(value.content_digest().as_str())
    }

    pub(crate) fn validated(&mut self, value: &ValidatedCanonicalValue) -> Result<()> {
        self.string(value.schema_contract())?;
        self.bytes(value.as_bytes())
    }

    pub(crate) fn schema_qualified(&mut self, value: &SchemaQualifiedCanonicalValue) -> Result<()> {
        self.string(value.schema_id().as_str())?;
        self.bytes(value.as_bytes())
    }

    pub(crate) fn finish(mut self) -> Result<Vec<u8>> {
        if self.bytes.len() > MAX_DURABLE_SNAPSHOT_BYTES - CHECKSUM_BYTES {
            return Err(ExecutorError::InvalidDurableSnapshot);
        }
        let digest = sha256_digest_bytes(&self.bytes);
        self.bytes.extend_from_slice(digest.as_bytes());
        Ok(self.bytes)
    }
}

pub(crate) struct Decoder<'a> {
    bytes: &'a [u8],
    offset: usize,
    payload_end: usize,
}

impl<'a> Decoder<'a> {
    pub(crate) fn new(bytes: &'a [u8], magic: &[u8; 8]) -> Result<Self> {
        if bytes.len() < magic.len() + CHECKSUM_BYTES
            || bytes.len() > MAX_DURABLE_SNAPSHOT_BYTES
            || &bytes[..magic.len()] != magic
        {
            return Err(ExecutorError::InvalidDurableSnapshot);
        }
        let payload_end = bytes.len() - CHECKSUM_BYTES;
        let expected = sha256_digest_bytes(&bytes[..payload_end]);
        if expected.as_bytes() != &bytes[payload_end..] {
            return Err(ExecutorError::InvalidDurableSnapshot);
        }
        Ok(Self {
            bytes,
            offset: magic.len(),
            payload_end,
        })
    }

    pub(crate) fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn u32(&mut self) -> Result<u32> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| ExecutorError::InvalidDurableSnapshot)?;
        Ok(u32::from_be_bytes(bytes))
    }

    pub(crate) fn u64(&mut self) -> Result<u64> {
        let bytes: [u8; 8] = self
            .take(8)?
            .try_into()
            .map_err(|_| ExecutorError::InvalidDurableSnapshot)?;
        Ok(u64::from_be_bytes(bytes))
    }

    pub(crate) fn count(
        &mut self,
        contract_maximum: usize,
        minimum_encoded_item_bytes: usize,
    ) -> Result<usize> {
        let value =
            usize::try_from(self.u64()?).map_err(|_| ExecutorError::InvalidDurableSnapshot)?;
        let byte_maximum = self
            .remaining()
            .checked_div(minimum_encoded_item_bytes.max(1))
            .ok_or(ExecutorError::InvalidDurableSnapshot)?;
        if value > contract_maximum || value > byte_maximum {
            return Err(ExecutorError::InvalidDurableSnapshot);
        }
        Ok(value)
    }

    pub(crate) fn string(&mut self) -> Result<String> {
        let bytes = self.length_prefixed(MAX_STRING_BYTES)?;
        let value =
            std::str::from_utf8(bytes).map_err(|_| ExecutorError::InvalidDurableSnapshot)?;
        Ok(value.to_owned())
    }

    pub(crate) fn canonical_bytes(&mut self) -> Result<Vec<u8>> {
        Ok(self.length_prefixed(MAX_DURABLE_SNAPSHOT_BYTES)?.to_vec())
    }

    pub(crate) fn content_ref(&mut self) -> Result<ContentRef> {
        let schema_id = SchemaId::from_str(&self.string()?)
            .map_err(|_| ExecutorError::InvalidDurableSnapshot)?;
        let content_digest = ContentDigest::from_str(&self.string()?)
            .map_err(|_| ExecutorError::InvalidDurableSnapshot)?;
        ContentRef::new(schema_id, content_digest)
            .map_err(|_| ExecutorError::InvalidDurableSnapshot)
    }

    pub(crate) fn validated(&mut self) -> Result<ValidatedCanonicalValue> {
        let schema_contract = self.string()?;
        let bytes = self.canonical_bytes()?;
        recoverability_contract()?
            .strict_decode(&schema_contract, &bytes)
            .map_err(contract_error)
    }

    pub(crate) fn schema_qualified(&mut self) -> Result<SchemaQualifiedCanonicalValue> {
        let schema_id = SchemaId::from_str(&self.string()?)
            .map_err(|_| ExecutorError::InvalidDurableSnapshot)?;
        let bytes = self.canonical_bytes()?;
        SchemaQualifiedCanonicalValue::new(schema_id, &bytes)
            .map_err(|_| ExecutorError::InvalidDurableSnapshot)
    }

    pub(crate) fn request_digest(&mut self) -> Result<RequestDigest> {
        RequestDigest::parse(self.string()?).map_err(|_| ExecutorError::InvalidDurableSnapshot)
    }

    pub(crate) fn effect_key(&mut self) -> Result<EffectKey> {
        EffectKey::parse(self.string()?).map_err(|_| ExecutorError::InvalidDurableSnapshot)
    }

    pub(crate) fn attempt_id(&mut self) -> Result<AttemptId> {
        AttemptId::parse(self.string()?).map_err(|_| ExecutorError::InvalidDurableSnapshot)
    }

    pub(crate) fn tenant_scope_id(&mut self) -> Result<TenantScopeId> {
        TenantScopeId::from_str(&self.string()?).map_err(|_| ExecutorError::InvalidDurableSnapshot)
    }

    pub(crate) fn remaining(&self) -> usize {
        self.payload_end.saturating_sub(self.offset)
    }

    pub(crate) fn finish(self) -> Result<()> {
        if self.offset != self.payload_end {
            return Err(ExecutorError::InvalidDurableSnapshot);
        }
        Ok(())
    }

    fn length_prefixed(&mut self, maximum: usize) -> Result<&'a [u8]> {
        let len =
            usize::try_from(self.u64()?).map_err(|_| ExecutorError::InvalidDurableSnapshot)?;
        if len > maximum || len > self.remaining() {
            return Err(ExecutorError::InvalidDurableSnapshot);
        }
        self.take(len)
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or(ExecutorError::InvalidDurableSnapshot)?;
        if end > self.payload_end {
            return Err(ExecutorError::InvalidDurableSnapshot);
        }
        let bytes = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(bytes)
    }
}
