//! Deterministic per-head append request identities.

use mfm_ids::{AppendRequestId, NodeId};
use mfm_journal::v1::JournalHead;

use crate::{Result, RuntimeError};

pub(crate) fn append_request_id(
    action: &'static str,
    head: &JournalHead,
    node_id: Option<&NodeId>,
) -> Result<AppendRequestId> {
    let head = head.fields()?;
    let digest = suffix(head.commit_digest.as_str())?;
    let value = match node_id {
        Some(node_id) => {
            format!("runtime/{action}/{digest}/{}", suffix(node_id.as_str())?)
        }
        None => format!("runtime/{action}/{digest}"),
    };
    AppendRequestId::new(value).map_err(|_| RuntimeError::InvalidCallbackResult)
}

fn suffix(value: &str) -> Result<&str> {
    value
        .rsplit(':')
        .next()
        .filter(|value| !value.is_empty())
        .ok_or(RuntimeError::InvalidCallbackResult)
}

#[cfg(test)]
mod tests {
    use mfm_ids::{DigestAlgorithm, DigestBytes, JournalCommitDigest, NodeId};

    use super::*;

    #[test]
    fn identities_are_stable_for_one_head_and_change_with_the_head() {
        let digest = |byte| DigestBytes::from_array([byte; 32]);
        let first_head = JournalHead::new(1, &JournalCommitDigest::from_digest(digest(1))).unwrap();
        let second_head =
            JournalHead::new(2, &JournalCommitDigest::from_digest(digest(2))).unwrap();
        let node = NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(3));

        let first = append_request_id("pure", &first_head, Some(&node)).unwrap();
        assert_eq!(
            first,
            append_request_id("pure", &first_head, Some(&node)).unwrap()
        );
        assert_ne!(
            first,
            append_request_id("pure", &second_head, Some(&node)).unwrap()
        );
    }
}
