Implementation Plan

  Core direction: make manual resolution a certified, signed authorization decision. No compatibility layer. Old operator_identity_ref_* manual events/
  specs become invalid.

  Target Shape

  ManualResolutionEvidenceSpec becomes:

  pub struct ManualResolutionEvidenceSpec {
      pub evidence_schema: SchemaId,
      pub authorization: ManualResolutionAuthorizationSpec,
  }

  pub struct ManualResolutionAuthorizationSpec {
      pub verifier_id: ManualAuthorizationVerifierId,
      pub signing_scheme: ManualSigningSchemeSpec,
      pub authority: OperatorAuthoritySnapshotSpec,
      pub quorum: ManualAuthorizationQuorumSpec,
  }

  ManualResolutionRecorded becomes evidence + authorization proof refs:

  pub struct ManualResolutionRecorded {
      pub run_id: RunId,
      pub spec_hash: SpecHash,
      pub outcome: ManualResolutionOutcome,
      pub evidence_schema_id: SchemaId,
      pub evidence_hash: ContentDigest,
      pub evidence_artifact_id: ArtifactId,
      pub authorization_schema_id: SchemaId,
      pub authorization_hash: ContentDigest,
      pub authorization_artifact_id: ArtifactId,
      pub note: Option<ManualResolutionNote>,
  }

  The proof artifact contains a canonical claim plus signature. The claim binds run_id, spec_hash, expected next seq, stream prefix digest, manual
  block reason, unresolved obligations digest, outcome, and evidence artifact hash/id/schema. `ManuallyResolved` means an authorized manual decision
  was recorded; it is not an independent platform proof of domain truth.

  Authority split:

  - certification uses the registry as live authority for schema roles, manual verifier identities, and operator authority snapshots;
  - the certified spec and certificate carry replay authority;
  - replay verifies from certified spec/certificate, stream, and artifacts only;
  - replay never calls live signer, verifier registry, certification registry, keystore, or runtime signer sources.

  Commit Sequence

  1. docs: define signed manual authorization authority

  Update RFC_ACDC_SAGA.md, DESIGN_ACDC.md, and implementation plan notes.

  Decisions to lock:

  - manual resolution requires certified schema authority plus signed authorization proof;
  - registry is used at certification time;
  - certificate/spec carry replay authority;
  - replay never calls live signer/registry;
  - ManuallyResolved means “authorized manual decision recorded,” not “domain truth independently proven.”

  2. spec: replace manual operator schema with authorization policy

  Update mfm-spec:

  - remove operator_identity_ref_schema;
  - add manual authorization spec types;
  - add operator identity and authority snapshot newtypes;
  - add canonical JSON/hashing/parsing;
  - deny old fields through closed parsing.

  Type-safety goal: no ManualResolutionEvidenceSpec can exist without authorization policy.

  3. events: require manual authorization proof refs

  Update mfm-events and store codecs:

  - change ManualResolutionRecorded;
  - add ArtifactRole::ManualResolutionEvidence;
  - add ArtifactRole::ManualResolutionAuthorization;
  - update event schemas and artifact requirement derivation.

  Type-safety goal: manual event shape cannot carry loose operator identity artifacts anymore.

  4. certify: add schema and manual authority evidence

  Extend CertificationRegistry:

  - trusted schema entries with roles;
  - trusted manual authorization verifier identities;
  - trusted operator authority snapshots.

  Extend certificate evidence:

  - certified schema role grants;
  - manual verifier evidence;
  - authority snapshot digest/evidence.

  Certification rejects:

  - unknown manual evidence schema;
  - schema without ManualResolutionEvidence role;
  - unknown verifier;
  - authority snapshot not present in trusted registry;
  - empty operator set;
  - unsupported quorum/signing scheme.

  5. manual-auth: add canonical claim and verifier contract

  Add a small shared pure contract module/crate for:

  - ManualResolutionAuthorizationClaim;
  - ManualResolutionAuthorizationProof;
  - canonical claim digest;
  - ManualAuthorizationVerifier trait;
  - verifier registry;
  - VerifiedManualResolution sealed result.

  Type-safety goal: only a verifier can mint VerifiedManualResolution.

  6. signing: add manual-resolution signing purpose

  Add a dedicated digest-only signing domain/purpose, not arbitrary message signing.

  The signer request remains transient. Persist only the resulting proof artifact. Keep the existing compile-fail test that
  SigningRequest::from_message does not exist.

  7. runtime: build manual authorization commits from verified proof

  Runtime/app admission flow:

  - derive prefix ManualBlocked;
  - compute prefix digest and unresolved-obligations digest;
  - build claim;
  - sign claim digest;
  - verify signer against certified authority;
  - stage evidence + authorization proof artifacts;
  - append ManualResolutionRecorded.

  Type-safety goal: expose a high-level API that accepts VerifiedManualResolution, not raw event fields.

  8. replay: verify signed manual authorization

  Replay rejects unless:

  - prefix derives ManualBlocked;
  - event matches certified policy;
  - evidence artifact matches digest/schema/role;
  - authorization artifact matches digest/schema/role;
  - claim matches event, prefix, outcome, evidence, obligations;
  - signature verifies;
  - signer is allowed by certified authority;
  - quorum is satisfied.

  Also fix runtime history so ManualResolutionRecorded is not ignored or over-trusted before verification.

  9. app: expose manual authority requirements

  Status/API should show:

  - required evidence schema;
  - signing scheme;
  - authority id;
  - allowed operator public identities or a redacted/summary view;
  - quorum.

  No secrets, signer runtime sources, keystore paths, or password/env details.

  10. tests: cover signed manual resolution authority

  Essential tests:

  - certification rejects unknown manual evidence schema;
  - certification rejects schema with wrong role;
  - certification rejects unknown verifier;
  - certification rejects authority snapshot mismatch;
  - certification rejects empty operator authority;
  - old manual spec/event JSON fails parse;
  - store rejects manual artifacts with wrong role/schema/digest;
  - replay accepts valid signed manual path;
  - replay rejects wrong run/spec/prefix/outcome/evidence hash;
  - replay rejects stale prefix sequence;
  - replay rejects signer not in authority snapshot;
  - replay rejects invalid signature;
  - replay rejects missing authorization artifact;
  - runtime cannot append manual resolution before ManualBlocked;
  - resume rejects unauthorized historical manual event;
  - UI/trybuild test: arbitrary-message signing still unavailable.

  Verification per slice: focused cargo test -p mfm-spec, mfm-events, mfm-certify, mfm-store, mfm-runtime, mfm-replay, mfm-app, plus cargo fmt --all --
  --check.
