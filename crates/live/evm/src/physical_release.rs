//! Immutable public physical-binding release histories.

use std::collections::BTreeSet;

use mfm_ids::{ContentDigest, ContentRef};
use mfm_journal::structured::{AccessKind, HistoryObject};
use serde::Serialize;

use crate::EvmStructuredLiveBindingError;

/// One immutable public release of an exact physical target.
///
/// The certificate's content reference is the release identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmPhysicalBindingRelease {
    admitted_routing_policy_ref: ContentRef,
    physical_target_ref: ContentRef,
    certificate: HistoryObject,
    predecessor_binding_ref: Option<ContentRef>,
    activation_lineage_head_ref: Option<ContentRef>,
}

impl EvmPhysicalBindingRelease {
    /// Constructs one exact public release relation.
    pub fn new(
        admitted_routing_policy_ref: ContentRef,
        physical_target_ref: ContentRef,
        certificate: HistoryObject,
        predecessor_binding_ref: Option<ContentRef>,
        activation_lineage_head_ref: Option<ContentRef>,
    ) -> Result<Self, EvmStructuredLiveBindingError> {
        certificate
            .validate()
            .map_err(|_| EvmStructuredLiveBindingError::InvalidContract)?;
        Ok(Self {
            admitted_routing_policy_ref,
            physical_target_ref,
            certificate,
            predecessor_binding_ref,
            activation_lineage_head_ref,
        })
    }

    /// Returns the routing policy admitted with this release.
    pub const fn admitted_routing_policy_ref(&self) -> &ContentRef {
        &self.admitted_routing_policy_ref
    }

    /// Returns the exact secret-free physical target identity.
    pub const fn physical_target_ref(&self) -> &ContentRef {
        &self.physical_target_ref
    }

    /// Returns the exact immutable release certificate.
    pub const fn certificate(&self) -> &HistoryObject {
        &self.certificate
    }

    /// Returns the exact preceding release certificate, when this is a successor.
    pub const fn predecessor_binding_ref(&self) -> Option<&ContentRef> {
        self.predecessor_binding_ref.as_ref()
    }

    /// Returns the public lineage head that activated this successor release.
    pub const fn activation_lineage_head_ref(&self) -> Option<&ContentRef> {
        self.activation_lineage_head_ref.as_ref()
    }
}

/// Complete ordered append-only release history for one physical purpose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmPhysicalBindingReleaseHistory {
    root: EvmPhysicalBindingRelease,
    successors: Vec<EvmPhysicalBindingRelease>,
}

impl EvmPhysicalBindingReleaseHistory {
    /// Returns the canonical digest of every ordered release field in this history.
    pub fn content_digest(&self) -> Result<ContentDigest, EvmStructuredLiveBindingError> {
        #[derive(Serialize)]
        struct Release<'a> {
            admitted_routing_policy_ref: &'a ContentRef,
            physical_target_ref: &'a ContentRef,
            certificate_ref: &'a ContentRef,
            predecessor_binding_ref: Option<&'a ContentRef>,
            activation_lineage_head_ref: Option<&'a ContentRef>,
        }
        let releases = self
            .releases()
            .map(|release| Release {
                admitted_routing_policy_ref: release.admitted_routing_policy_ref(),
                physical_target_ref: release.physical_target_ref(),
                certificate_ref: &release.certificate().content_ref,
                predecessor_binding_ref: release.predecessor_binding_ref(),
                activation_lineage_head_ref: release.activation_lineage_head_ref(),
            })
            .collect::<Vec<_>>();
        mfm_journal::structured::domain_content_digest(
            "mfm.evm.physical-binding-release-history.v1",
            &releases,
        )
        .map_err(|_| EvmStructuredLiveBindingError::InvalidContract)
    }

    /// Constructs and validates one non-empty linear release history.
    pub fn new(
        releases: Vec<EvmPhysicalBindingRelease>,
    ) -> Result<Self, EvmStructuredLiveBindingError> {
        let mut certificate_refs = BTreeSet::new();
        for (index, release) in releases.iter().enumerate() {
            release
                .certificate
                .validate()
                .map_err(|_| EvmStructuredLiveBindingError::InvalidContract)?;
            if !certificate_refs.insert(release.certificate.content_ref.clone()) {
                return Err(EvmStructuredLiveBindingError::InvalidContract);
            }
            let expected_predecessor = index
                .checked_sub(1)
                .and_then(|previous| releases.get(previous))
                .map(|previous| &previous.certificate.content_ref);
            if release.predecessor_binding_ref.as_ref() != expected_predecessor {
                return Err(EvmStructuredLiveBindingError::InvalidContract);
            }
        }
        let mut releases = releases.into_iter();
        let root = releases
            .next()
            .ok_or(EvmStructuredLiveBindingError::InvalidContract)?;
        Ok(Self {
            root,
            successors: releases.collect(),
        })
    }

    /// Constructs a one-release history with no predecessor or activation head.
    pub fn single(
        admitted_routing_policy_ref: ContentRef,
        physical_target_ref: ContentRef,
        certificate: HistoryObject,
    ) -> Result<Self, EvmStructuredLiveBindingError> {
        Self::new(vec![EvmPhysicalBindingRelease::new(
            admitted_routing_policy_ref,
            physical_target_ref,
            certificate,
            None,
            None,
        )?])
    }

    /// Returns every retained release in strict predecessor order.
    pub fn releases(&self) -> impl DoubleEndedIterator<Item = &EvmPhysicalBindingRelease> {
        std::iter::once(&self.root).chain(self.successors.iter())
    }

    /// Returns the number of immutable retained releases.
    pub const fn release_count(&self) -> usize {
        self.successors.len() + 1
    }

    /// Returns the exact current release.
    pub fn current(&self) -> &EvmPhysicalBindingRelease {
        self.successors.last().unwrap_or(&self.root)
    }

    /// Finds one retained release by exact certificate identity.
    pub fn release(&self, binding_ref: &ContentRef) -> Option<&EvmPhysicalBindingRelease> {
        self.releases()
            .find(|release| &release.certificate.content_ref == binding_ref)
    }

    /// Reports whether `candidate` is a strict retained descendant of `ancestor`.
    pub fn is_strict_descendant(&self, candidate: &ContentRef, ancestor: &ContentRef) -> bool {
        let ancestor = self
            .releases()
            .position(|release| &release.certificate.content_ref == ancestor);
        let candidate = self
            .releases()
            .position(|release| &release.certificate.content_ref == candidate);
        matches!((ancestor, candidate), (Some(ancestor), Some(candidate)) if candidate > ancestor)
    }
}

/// Exact capability, adapter implementation, target, and release-history purpose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmPhysicalBindingPurpose {
    access_kind: AccessKind,
    capability_contract_ref: ContentRef,
    adapter_contract_ref: ContentRef,
    adapter_implementation_ref: ContentRef,
    stable_resource_lineage_contract_ref: Option<ContentRef>,
    release_history: EvmPhysicalBindingReleaseHistory,
}

impl EvmPhysicalBindingPurpose {
    /// Constructs one exact purpose and validates its release semantics.
    pub fn new(
        access_kind: AccessKind,
        capability_contract_ref: ContentRef,
        adapter_contract_ref: ContentRef,
        adapter_implementation_ref: ContentRef,
        stable_resource_lineage_contract_ref: Option<ContentRef>,
        release_history: EvmPhysicalBindingReleaseHistory,
    ) -> Result<Self, EvmStructuredLiveBindingError> {
        let valid_lineage = match access_kind {
            AccessKind::Read => {
                stable_resource_lineage_contract_ref.is_none()
                    && release_history
                        .releases()
                        .all(|release| release.activation_lineage_head_ref().is_none())
            }
            AccessKind::Effect => {
                stable_resource_lineage_contract_ref.is_some()
                    && release_history
                        .releases()
                        .enumerate()
                        .all(|(index, release)| {
                            (index == 0) == release.activation_lineage_head_ref().is_none()
                        })
            }
        };
        let unique_activation_heads = access_kind == AccessKind::Read
            || release_history
                .releases()
                .filter_map(EvmPhysicalBindingRelease::activation_lineage_head_ref)
                .collect::<BTreeSet<_>>()
                .len()
                == release_history.release_count().saturating_sub(1);
        if !valid_lineage || !unique_activation_heads {
            return Err(EvmStructuredLiveBindingError::InvalidContract);
        }
        Ok(Self {
            access_kind,
            capability_contract_ref,
            adapter_contract_ref,
            adapter_implementation_ref,
            stable_resource_lineage_contract_ref,
            release_history,
        })
    }

    /// Returns this purpose's access kind.
    pub const fn access_kind(&self) -> AccessKind {
        self.access_kind
    }

    /// Returns the exact semantic capability contract.
    pub const fn capability_contract_ref(&self) -> &ContentRef {
        &self.capability_contract_ref
    }

    /// Returns the exact semantic adapter contract.
    pub const fn adapter_contract_ref(&self) -> &ContentRef {
        &self.adapter_contract_ref
    }

    /// Returns the exact qualified adapter implementation.
    pub const fn adapter_implementation_ref(&self) -> &ContentRef {
        &self.adapter_implementation_ref
    }

    /// Returns the stable resource lineage for an Effect purpose.
    pub const fn stable_resource_lineage_contract_ref(&self) -> Option<&ContentRef> {
        self.stable_resource_lineage_contract_ref.as_ref()
    }

    /// Returns the complete immutable release history.
    pub const fn release_history(&self) -> &EvmPhysicalBindingReleaseHistory {
        &self.release_history
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_canonical::sha256_digest_bytes;
    use mfm_ids::{DigestAlgorithm, SchemaId, StableId};

    #[test]
    fn release_history_requires_the_complete_exact_predecessor_chain() {
        let policy_ref = object(1).content_ref;
        let first_certificate = object(2);
        let second_certificate = object(3);
        let activation_head_ref = object(4).content_ref;
        let valid = EvmPhysicalBindingReleaseHistory::new(vec![
            EvmPhysicalBindingRelease::new(
                policy_ref.clone(),
                object(5).content_ref,
                first_certificate.clone(),
                None,
                None,
            )
            .expect("root release"),
            EvmPhysicalBindingRelease::new(
                policy_ref.clone(),
                object(6).content_ref,
                second_certificate.clone(),
                Some(first_certificate.content_ref.clone()),
                Some(activation_head_ref.clone()),
            )
            .expect("successor release"),
        ])
        .expect("complete release history");
        assert_eq!(valid.release_count(), 2);
        assert!(valid.is_strict_descendant(
            &second_certificate.content_ref,
            &first_certificate.content_ref
        ));

        let omitted_predecessor = EvmPhysicalBindingReleaseHistory::new(vec![
            EvmPhysicalBindingRelease::new(
                policy_ref.clone(),
                object(5).content_ref,
                first_certificate.clone(),
                None,
                None,
            )
            .expect("root release"),
            EvmPhysicalBindingRelease::new(
                policy_ref,
                object(6).content_ref,
                second_certificate,
                None,
                Some(activation_head_ref),
            )
            .expect("malformed successor release"),
        ]);
        assert_eq!(
            omitted_predecessor,
            Err(EvmStructuredLiveBindingError::InvalidContract)
        );
    }

    #[test]
    fn history_digest_binds_predecessor_and_target_beyond_the_current_certificate() {
        let policy = object(20).content_ref;
        let current_certificate = object(21);
        let first_root = object(22);
        let second_root = object(23);
        let history = |root: HistoryObject, target_seed: u8| {
            EvmPhysicalBindingReleaseHistory::new(vec![
                EvmPhysicalBindingRelease::new(
                    policy.clone(),
                    object(target_seed).content_ref,
                    root.clone(),
                    None,
                    None,
                )
                .expect("root release"),
                EvmPhysicalBindingRelease::new(
                    policy.clone(),
                    object(24).content_ref,
                    current_certificate.clone(),
                    Some(root.content_ref),
                    None,
                )
                .expect("current release"),
            ])
            .expect("release history")
        };
        let first = history(first_root, 25);
        let second = history(second_root, 26);
        assert_eq!(
            first.current().certificate().content_ref,
            second.current().certificate().content_ref
        );
        assert_ne!(
            first.content_digest().expect("first digest"),
            second.content_digest().expect("second digest")
        );
    }

    #[test]
    fn read_history_accepts_a_same_target_implementation_successor_and_changes_digest() {
        let policy = object(70).content_ref;
        let target = object(71).content_ref;
        let v1_certificate = object(72);
        let v2_certificate = object(73);
        let v1 = EvmPhysicalBindingReleaseHistory::single(
            policy.clone(),
            target.clone(),
            v1_certificate.clone(),
        )
        .expect("v1 signer release");
        let v2 = EvmPhysicalBindingReleaseHistory::new(vec![
            EvmPhysicalBindingRelease::new(
                policy.clone(),
                target.clone(),
                v1_certificate.clone(),
                None,
                None,
            )
            .expect("retained v1 signer release"),
            EvmPhysicalBindingRelease::new(
                policy,
                target.clone(),
                v2_certificate.clone(),
                Some(v1_certificate.content_ref.clone()),
                None,
            )
            .expect("v2 signer successor"),
        ])
        .expect("append-only signer release history");

        assert_eq!(v2.release_count(), 2);
        assert!(v2
            .releases()
            .all(|release| release.physical_target_ref() == &target));
        assert_eq!(
            v2.current().certificate().content_ref,
            v2_certificate.content_ref
        );
        assert!(v2.is_strict_descendant(&v2_certificate.content_ref, &v1_certificate.content_ref));
        assert_ne!(
            v1.content_digest().expect("v1 release-history digest"),
            v2.content_digest().expect("v2 release-history digest")
        );
        EvmPhysicalBindingPurpose::new(
            AccessKind::Read,
            object(74).content_ref,
            object(75).content_ref,
            object(76).content_ref,
            None,
            v2,
        )
        .expect("same-target Read implementation successor");
    }

    fn object(discriminator: u8) -> HistoryObject {
        HistoryObject::new(
            StableId::new(format!("mfm.evm-live.test/object-{discriminator}"))
                .expect("test object type"),
            SchemaId::new(
                "mfm.evm-live.test-object",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                sha256_digest_bytes(b"mfm.evm-live.test-object.v1"),
            )
            .expect("test schema"),
            format!("{{\"discriminator\":{discriminator}}}"),
        )
        .expect("test object")
    }
}
