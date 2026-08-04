#!/usr/bin/env python3
"""Generate the one current structured recoverability annex and corpus."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent / "v1"


def canonical(value: Any) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        separators=(",", ":"),
        sort_keys=True,
    ).encode()


def string(grammar: str, minimum: int, maximum: int) -> dict[str, Any]:
    return {
        "grammar": grammar,
        "kind": "string",
        "max_length": maximum,
        "min_length": minimum,
    }


def enum(*values: str) -> dict[str, Any]:
    return {
        "kind": "string",
        "max_length": max(map(len, values)),
        "min_length": min(map(len, values)),
        "values": list(values),
    }


def tagged_union(*variants: tuple[str, list[dict[str, Any]]]) -> dict[str, Any]:
    return {
        "discriminator": "kind",
        "invariants": [],
        "kind": "tagged_union",
        "unknown_fields": "reject",
        "variants": [
            {"fields": fields, "tag": tag}
            for tag, fields in variants
        ],
    }


def reference(contract: str) -> dict[str, str]:
    return {"contract": contract, "kind": "reference"}


def field(name: str, shape: dict[str, Any], *, optional: bool = False) -> dict[str, Any]:
    if optional:
        shape = {"kind": "optional_absent", "value": shape}
    return {
        "name": name,
        "presence": "optional_absent" if optional else "required",
        "type": shape,
    }


def object_shape(fields: list[dict[str, Any]], *invariants: str) -> dict[str, Any]:
    return {
        "fields": fields,
        "invariants": list(invariants),
        "kind": "object",
        "unknown_fields": "reject",
    }


def array(
    items: dict[str, Any],
    minimum: int = 0,
    maximum: int = 1_048_576,
    *,
    unique: bool = False,
    ordering: str = "preserved",
) -> dict[str, Any]:
    return {
        "items": items,
        "kind": "array",
        "max_items": maximum,
        "min_items": minimum,
        "ordering": ordering,
        "unique": unique,
    }


def unsigned(maximum: int, minimum: int = 0) -> dict[str, Any]:
    return {
        "kind": "bounded_unsigned_integer",
        "maximum": str(maximum),
        "minimum": str(minimum),
        "wire": "json_integer",
    }


def nullable(shape: dict[str, Any]) -> dict[str, Any]:
    return {"kind": "nullable", "value": shape}


def literal(value: Any) -> dict[str, Any]:
    return {"kind": "literal", "value": value}


def native_canonical_shape() -> dict[str, Any]:
    return {
        "discriminator": "kind",
        "discriminator_presence": "derived_from_native_json_type_not_encoded",
        "invariants": [
            "wire is one unwrapped native float-free JSON value",
            "object keys are unique and encoded in RFC 8785 UTF-16 order",
        ],
        "kind": "tagged_union",
        "unknown_fields": "reject",
        "variants": [
            {"fields": [], "tag": "null"},
            {"fields": [field("value", {"kind": "boolean"})], "tag": "boolean"},
            {
                "fields": [field("value", string("valid_unicode_scalar_string", 0, 16_777_216))],
                "tag": "string",
            },
            {"fields": [field("value", unsigned(18_446_744_073_709_551_615))], "tag": "unsigned"},
            {
                "fields": [
                    field(
                        "value",
                        string("-?[0-9]+ with no leading zeroes or negative zero", 1, 20),
                    )
                ],
                "tag": "signed",
            },
            {
                "fields": [field("items", array(reference("mfm.primitive-canonical_value.v1")))],
                "tag": "array",
            },
            {
                "fields": [
                    field(
                        "entries",
                        array(
                            reference("mfm.primitive-canonical_object_entry.v1"),
                            unique=True,
                            ordering="utf16_key",
                        ),
                    )
                ],
                "tag": "object",
            },
        ],
        "wire_representation": "unwrapped_native_float_free_json",
    }


def schema_shapes() -> dict[str, tuple[list[str], dict[str, Any]]]:
    canonical_value = reference("mfm.primitive-canonical_value.v1")
    content_ref = reference("mfm.content-ref.v1")
    semantic_digest = reference("mfm.primitive-semantic_digest.v1")
    journal_head = reference("mfm.journal-head.v1")
    schemas: dict[str, tuple[list[str], dict[str, Any]]] = {
        "mfm.primitive-canonical_value.v1": (["P-HB-01"], native_canonical_shape()),
        "mfm.primitive-canonical_object_entry.v1": (
            ["P-HB-01"],
            object_shape(
                [
                    field("key", string("valid_unicode_scalar_string", 0, 1_048_576)),
                    field("value", canonical_value),
                ]
            ),
        ),
        "mfm.primitive-stable_id.v1": (
            ["P-HB-01"],
            string("[a-z0-9][a-z0-9._/-]*", 1, 512),
        ),
        "mfm.primitive-entry_point_id.v1": (
            ["P-AP-01"],
            string("mfm.<stable-domain>/<stable-name>@<positive-canonical-u64>", 9, 512),
        ),
        "mfm.primitive-invocation_uuid_v4.v1": (
            ["P-AP-01"],
            string(
                "[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}",
                36,
                36,
            ),
        ),
        "mfm.primitive-content_digest.v1": (
            ["P-CA-01"],
            string("content:sha256-v1:[0-9a-f]{64}", 82, 82),
        ),
        "mfm.primitive-schema_id.v1": (
            ["P-HB-01"],
            string("schema:<name>:<positive-version>:sha256-jcs-v1:[0-9a-f]{64}", 90, 512),
        ),
        "mfm.primitive-semantic_digest.v1": (
            ["P-HB-01"],
            string("sha256-jcs-v1:[0-9a-f]{64}", 78, 78),
        ),
        "mfm.primitive-run_id.v1": (
            ["P-RH-01"],
            string("run:sha256-jcs-v1:[0-9a-f]{64}", 82, 82),
        ),
        "mfm.primitive-occurrence_id.v1": (
            ["P-AP-01"],
            string("occurrence:sha256-jcs-v1:[0-9a-f]{64}", 89, 89),
        ),
        "mfm.primitive-store_scope_id.v1": (
            ["P-RH-01"],
            string("mfm.store_scope.v1:[0-9a-f]{32}", 51, 51),
        ),
        "mfm.primitive-store_epoch.v1": (
            ["P-RH-01"],
            string("0|[1-9][0-9]{0,19}", 1, 20),
        ),
        "mfm.primitive-tenant_scope_id.v1": (
            ["P-RH-01"],
            string("mfm.tenant_scope.v1:[0-9a-f]{32}", 52, 52),
        ),
        "mfm.primitive-semantic_identity.v1": (
            ["P-HB-01"],
            string("versioned semantic identity with sha256-jcs-v1 digest", 78, 512),
        ),
        "mfm.primitive-media_type.v1": (
            ["P-HB-01"],
            string("lowercase registered media type without parameters", 3, 256),
        ),
        "mfm.content-ref.v1": (
            ["P-CA-01"],
            object_shape(
                [
                    field("content_digest", reference("mfm.primitive-content_digest.v1")),
                    field("schema_id", reference("mfm.primitive-schema_id.v1")),
                ],
                "content digest uses sha256-v1 over exact retained bytes",
            ),
        ),
        "mfm.journal-head.v1": (
            ["P-RH-01"],
            object_shape(
                [
                    field("commit_digest", semantic_digest),
                    field("run_sequence", unsigned(18_446_744_073_709_551_615, 1)),
                ],
                "run sequence is the one-based atomic append coordinate",
            ),
        ),
        "mfm.domain-separated-preimage.v1": (
            ["P-HB-01"],
            object_shape(
                [
                    field("domain", string("mfm.[a-z0-9._-]+.v[1-9][0-9]*", 8, 512)),
                    field("value", canonical_value),
                ]
            ),
        ),
        "mfm.schema-descriptor.v1": (["P-HB-01"], canonical_value),
        "mfm.component-object-evidence-contract.v1": (
            ["P-CF-01"],
            object_shape(
                [field("version", literal("mfm.component-object-evidence-contract.v1"))]
            ),
        ),
        "mfm.retained-value-contract.v1": (
            ["P-CF-01"],
            object_shape(
                [
                    field("evidence_contract_ref", content_ref),
                    field("media_type", reference("mfm.primitive-media_type.v1")),
                    field("role", reference("mfm.primitive-stable_id.v1")),
                    field("schema_id", reference("mfm.primitive-schema_id.v1")),
                    field("semantic_type_id", reference("mfm.primitive-semantic_identity.v1")),
                ]
            ),
        ),
        "mfm.fact-selection-query.v1": (
            ["P-FA-01"],
            object_shape(
                [
                    field("canonical_predicate", canonical_value),
                    field("content_identity_filter", semantic_digest, optional=True),
                    field("fact_descriptor_ref", content_ref),
                    field("limit", unsigned(128, 1)),
                    field("ordering", enum("ascending", "descending")),
                    field(
                        "tie_break",
                        enum("fact_identity_ascending", "fact_identity_descending"),
                    ),
                ]
            ),
        ),
        "mfm.structured-typed-value-ref.v1": (
            ["P-FA-01", "P-RH-01"],
            object_shape(
                [
                    field("contract_ref", content_ref),
                    field("value_ref", content_ref),
                ]
            ),
        ),
        "mfm.fact-content-identity-preimage.v1": (
            ["P-FA-01"],
            object_shape(
                [
                    field("fact_descriptor_ref", content_ref),
                    field("subject_ref", reference("mfm.structured-typed-value-ref.v1")),
                    field("response_ref", reference("mfm.structured-typed-value-ref.v1")),
                ],
                "producer coordinate and tenant fact order are excluded",
            ),
        ),
        "mfm.fact-logical-identity-preimage.v1": (
            ["P-FA-01"],
            object_shape(
                [
                    field("transition_ref", reference("mfm.structured-record-ref.v1")),
                    field("emission_ordinal", unsigned(4_294_967_295)),
                    field("fact_content_identity", semantic_digest),
                ],
                "tenant fact order is excluded",
            ),
        ),
        "mfm.fact-selection-request.v1": (
            ["P-FA-01"],
            object_shape(
                [
                    field("admitted_source_manifest_ref", content_ref),
                    field(
                        "completeness_mode",
                        literal("complete_through_authorization_frontier"),
                    ),
                    field("producer_scope", literal("other_runs_in_tenant_scope")),
                    field(
                        "queries",
                        array(reference("mfm.fact-selection-query.v1"), 1, 128),
                    ),
                    field(
                        "scan_bounds",
                        object_shape(
                            [
                                field("maximum_facts", unsigned(16_000_000, 1)),
                                field("maximum_publications", unsigned(1_000_000, 1)),
                                field("maximum_response_bytes", unsigned(16 * 1024 * 1024, 1)),
                                field(
                                    "maximum_retained_source_bytes",
                                    unsigned(512 * 1024 * 1024, 1),
                                ),
                                field("maximum_selected_results", unsigned(16_384, 1)),
                            ]
                        ),
                    ),
                    field("selector_contract_ref", content_ref),
                    field("version", literal("mfm.fact-selection-request.v1")),
                ],
                "selector_contract_ref is the one frozen prior-run fact selector contract",
                "all bounds are totals for the complete authorization-frontier scan",
            ),
        ),
        "mfm.planning-profile.v1": (
            ["P-CF-01"],
            object_shape(
                [
                    field("canonical_profile_parameters", canonical_value),
                    field(
                        "framework_policy_refs",
                        array(content_ref, 0, 128, unique=True),
                    ),
                    field("planner_contract_ref", content_ref),
                    field("planner_implementation_ref", content_ref),
                    field("version", literal("mfm.planning-profile.v1")),
                ],
                "framework policy references are unique and preserve declared order",
            ),
        ),
        "mfm.entry-point-contract.v1": (
            ["P-CF-01", "P-AP-01"],
            object_shape(
                [
                    field("entry_point_id", reference("mfm.primitive-entry_point_id.v1")),
                    field("entry_point_operation_id", reference("mfm.primitive-stable_id.v1")),
                    field("input_schema_id", reference("mfm.primitive-schema_id.v1")),
                    field("planning_profile", reference("mfm.planning-profile.v1")),
                    field("planning_profile_ref", content_ref),
                    field("public_output_schema_id", reference("mfm.primitive-schema_id.v1")),
                    field("version", literal("mfm.entry-point-contract.v1")),
                ],
                "planning_profile_ref binds the exact embedded profile bytes",
            ),
        ),
        "mfm.run-id-preimage.v1": (
            ["P-RH-01"],
            object_shape(
                [
                    field("entry_point_operation_id", reference("mfm.primitive-stable_id.v1")),
                    field("invocation_identity", reference("mfm.primitive-invocation_uuid_v4.v1")),
                    field("store_scope_id", reference("mfm.primitive-store_scope_id.v1")),
                    field("tenant_scope_id", reference("mfm.primitive-tenant_scope_id.v1")),
                ]
            ),
        ),
        "mfm.admit-run-request.v1": (
            ["P-AP-01"],
            object_shape(
                [
                    field("entry_point_id", reference("mfm.primitive-entry_point_id.v1")),
                    field("input", canonical_value),
                    field("invocation_identity", reference("mfm.primitive-invocation_uuid_v4.v1")),
                    field("version", literal("mfm.admit-run-request.v1")),
                ]
            ),
        ),
        "mfm.admit-run-response.v1": (
            ["P-AP-01"],
            object_shape(
                [
                    field("admission", enum("newly_admitted", "attached", "outcome_unknown")),
                    field("entry_point_id", reference("mfm.primitive-entry_point_id.v1")),
                    field("entry_point_operation_id", reference("mfm.primitive-stable_id.v1")),
                    field("invocation_identity", reference("mfm.primitive-invocation_uuid_v4.v1")),
                    field("planning_profile_ref", content_ref),
                    field("run_id", reference("mfm.primitive-run_id.v1")),
                    field("version", literal("mfm.admit-run-response.v1")),
                ]
            ),
        ),
        "mfm.drive-response.v1": (
            ["P-AP-01"],
            object_shape(
                [
                    field("journal_head", journal_head),
                    field("kind", enum("advanced", "waiting", "closed")),
                    field(
                        "reason",
                        nullable(
                            enum(
                                "retryable_evidence_gap",
                                "operational_block",
                                "integrity_block",
                            )
                        ),
                    ),
                    field("run_id", reference("mfm.primitive-run_id.v1")),
                ],
                "reason is present only as null or one reviewed waiting reason",
            ),
        ),
        "mfm.public-run-view.v1": (
            ["P-AP-01"],
            object_shape(
                [
                    field("entry_point_operation_id", reference("mfm.primitive-stable_id.v1")),
                    field("invocation_identity", reference("mfm.primitive-invocation_uuid_v4.v1")),
                    field("journal_head", journal_head),
                    field("outcome", nullable(canonical_value)),
                    field("run_id", reference("mfm.primitive-run_id.v1")),
                    field("semantic_head", canonical_value),
                    field(
                        "status",
                        enum(
                            "actionable",
                            "waiting_reads",
                            "possible_entry",
                            "blocked_integrity",
                            "closed",
                        ),
                    ),
                    field("tenant_scope_id", reference("mfm.primitive-tenant_scope_id.v1")),
                    field("version", literal("mfm.public-run-view.v1")),
                ]
            ),
        ),
        "mfm.public-runtime-fault-subject.v1": (
            ["P-AP-01"],
            tagged_union(
                (
                    "process",
                    [
                        field(
                            "component_kind",
                            enum("state", "capability", "adapter", "signer", "resource"),
                        ),
                        field("semantic_contract_ref", content_ref),
                    ],
                ),
                (
                    "store",
                    [
                        field("store_epoch", reference("mfm.primitive-store_epoch.v1")),
                        field("store_scope_id", reference("mfm.primitive-store_scope_id.v1")),
                    ],
                ),
            ),
        ),
        "mfm.public-runtime-fault-attribution.v1": (
            ["P-AP-01"],
            object_shape(
                [
                    field(
                        "occurrence_id",
                        nullable(reference("mfm.primitive-occurrence_id.v1")),
                    ),
                    field(
                        "phase",
                        enum(
                            "invoke_pure",
                            "author_request",
                            "qualify_access",
                            "settle_observation",
                            "qualify_candidate",
                            "append_candidate",
                            "load_history",
                            "resolve_append",
                        ),
                    ),
                    field("pre_fault_head", nullable(journal_head)),
                    field("run_id", reference("mfm.primitive-run_id.v1")),
                    field("subject", reference("mfm.public-runtime-fault-subject.v1")),
                ],
                "the process subject omits its private implementation identity",
                "the pre-fault head is null only when no journal head could be verified",
            ),
        ),
        "mfm.public-error.v1": (
            ["P-AP-01"],
            object_shape(
                [
                    field("code", string("valid_unicode_scalar_string", 1, 128)),
                    field("message", string("valid_unicode_scalar_string", 1, 4096)),
                    field(
                        "runtime_fault",
                        reference("mfm.public-runtime-fault-attribution.v1"),
                        optional=True,
                    ),
                ],
                "transport classification is carried by HTTP status or process exit status, not this body",
                "runtime_fault is present only for a Runtime-attributed failure",
            ),
        ),
        "mfm.error-response.v1": (
            ["P-AP-01"],
            object_shape(
                [
                    field("error", reference("mfm.public-error.v1")),
                    field("status", literal("error")),
                ],
                "CLI JSON and REST error bodies share this exact envelope",
            ),
        ),
        "mfm.replay-mode.v1": (
            ["P-AP-01"],
            enum("verify"),
        ),
        "mfm.structured-replay-result.v1": (
            ["P-AP-01", "P-RH-01"],
            object_shape(
                [
                    field("kind", literal("verified")),
                    field("journal_head", journal_head),
                    field("record_count", unsigned(18_446_744_073_709_551_615, 1)),
                    field("run_id", reference("mfm.primitive-run_id.v1")),
                    field("semantic_head", reference("mfm.structured-semantic-head.v1")),
                    field(
                        "status",
                        enum(
                            "actionable",
                            "waiting_reads",
                            "possible_entry",
                            "blocked_integrity",
                            "closed",
                        ),
                    ),
                    field("version", literal("mfm.structured-replay-result.v1")),
                ],
                "replay verification is a recorded-history summary and never a reproduction result",
            ),
        ),
        "mfm.structured-transition-trace.v1": (
            ["P-AP-01", "P-RH-01"],
            object_shape(
                [
                    field("after_semantic_state_digest", canonical_value),
                    field("before_semantic_state_digest", canonical_value),
                    field("consumed_observation_ref", nullable(canonical_value)),
                    field("facts", array(canonical_value)),
                    field("input", canonical_value),
                    field("occurrence_id", canonical_value),
                    field("occurrence_path_ref", content_ref),
                    field("outcome", canonical_value),
                    field("outcome_ref", content_ref),
                    field("record_ref", canonical_value),
                    field("semantic_call_id", canonical_value),
                    field("version", literal("mfm.structured-transition-trace.v1")),
                ],
                "trace entries expose only state-transition projection rows",
            ),
        ),
        "mfm.structured-access-audit.v1": (
            ["P-AP-01", "P-RH-01"],
            object_shape(
                [
                    field("access_attempt_id", canonical_value),
                    field("access_kind", enum("read", "effect")),
                    field("adapter_contract_ref", content_ref),
                    field("adapter_implementation_ref", content_ref),
                    field("attempt_ordinal", unsigned(18_446_744_073_709_551_615)),
                    field("authorization_ref", canonical_value),
                    field("capability_contract_ref", content_ref),
                    field("capability_implementation_ref", content_ref),
                    field("occurrence_id", canonical_value),
                    field("occurrence_path_ref", content_ref),
                    field("observation_ref", nullable(canonical_value)),
                    field("outcome", nullable(canonical_value)),
                    field("physical_binding_ref", content_ref),
                    field("request", canonical_value),
                    field("request_digest", canonical_value),
                    field("semantic_call_id", canonical_value),
                    field("stable_resource_lineage_contract_ref", nullable(content_ref)),
                    field(
                        "status",
                        enum(
                            "authorized",
                            "returned",
                            "safe_failure",
                            "superseded_before_entry",
                            "entry_unknown",
                            "integrity_fault",
                        ),
                    ),
                    field("version", literal("mfm.structured-access-audit.v1")),
                ],
                "audit entries expose authorization and observation identifiers without state outputs",
            ),
        ),
        "mfm.structured-record-ref.v1": (
            ["P-RH-01"],
            object_shape(
                [
                    field("ordinal", unsigned(4_294_967_295)),
                    field("record_hash", semantic_digest),
                    field("run_id", reference("mfm.primitive-run_id.v1")),
                    field("run_sequence", unsigned(18_446_744_073_709_551_615, 1)),
                ]
            ),
        ),
        "mfm.structured-semantic-head.v1": (
            ["P-RH-01"],
            tagged_union(
                (
                    "genesis",
                    [
                        field("admission_ref", reference("mfm.structured-record-ref.v1")),
                        field("semantic_state_digest", semantic_digest),
                    ],
                ),
                (
                    "transition",
                    [
                        field("semantic_state_digest", semantic_digest),
                        field("transition_ref", reference("mfm.structured-record-ref.v1")),
                    ],
                ),
            ),
        ),
        "mfm.structured-run-record.v1": (
            ["P-RH-01"],
            tagged_union(
                ("run_admitted", [field("payload", canonical_value)]),
                ("state_transition_committed", [field("payload", canonical_value)]),
                ("external_access_authorized", [field("payload", canonical_value)]),
                ("external_access_observed", [field("payload", canonical_value)]),
                ("run_closed", [field("payload", canonical_value)]),
            ),
        ),
        "mfm.structured-assigned-record.v1": (
            ["P-RH-01"],
            object_shape(
                [
                    field("record", reference("mfm.structured-run-record.v1")),
                    field("record_ref", reference("mfm.structured-record-ref.v1")),
                ]
            ),
        ),
        "mfm.structured-history-object.v1": (
            ["P-CA-01", "P-RH-01"],
            object_shape(
                [
                    field(
                        "canonical_json",
                        string("valid_unicode_scalar_string", 1, 16_777_216),
                    ),
                    field("content_ref", content_ref),
                    field("object_type", reference("mfm.primitive-stable_id.v1")),
                ],
                "content_ref hashes the exact canonical_json UTF-8 bytes",
            ),
        ),
        "mfm.portable-fixation.v1": (
            ["P-AP-01", "P-RH-01"],
            object_shape(
                [
                    field("journal_head", journal_head),
                    field(
                        "semantic_head",
                        reference("mfm.structured-semantic-head.v1"),
                    ),
                    field("store_epoch", reference("mfm.primitive-store_epoch.v1")),
                    field("store_scope_id", reference("mfm.primitive-store_scope_id.v1")),
                ],
                "semantic and physical fixation bind one exact export prefix",
            ),
        ),
        "mfm.portable-run-export-stream.v1": (
            ["P-AP-01", "P-RH-01"],
            object_shape(
                [
                    field(
                        "batches",
                        array(canonical_value, 1),
                    ),
                    field(
                        "digest",
                        reference("mfm.primitive-content_digest.v1"),
                    ),
                    field(
                        "fixation",
                        reference("mfm.portable-fixation.v1"),
                    ),
                    field("kind", enum("semantic", "audit")),
                    field("run_id", reference("mfm.primitive-run_id.v1")),
                    field(
                        "source_run_ids",
                        array(reference("mfm.primitive-run_id.v1")),
                    ),
                    field("store_scope_id", reference("mfm.primitive-store_scope_id.v1")),
                    field("tenant_scope_id", reference("mfm.primitive-tenant_scope_id.v1")),
                    field("version", literal("mfm.structured-portable-run-export.v1")),
                ],
                "the export is one exact canonical JSON object with committed-batch envelopes",
                "digest covers every required member except itself",
            ),
        ),
    }
    return schemas


def schema_id(contract: str, predicates: list[str], shape: dict[str, Any]) -> str:
    descriptor = {"contract": contract, "owner_predicates": predicates, "shape": shape}
    envelope = {"domain": "mfm.schema.v1", "value": descriptor}
    digest = hashlib.sha256(canonical(envelope)).hexdigest()
    name, version = contract.rsplit(".v", 1)
    return f"schema:{name}:{version}:sha256-jcs-v1:{digest}"


def build_annex() -> dict[str, Any]:
    schemas = []
    for contract, (predicates, shape) in sorted(schema_shapes().items()):
        schemas.append(
            {
                "contract": contract,
                "owner_predicates": predicates,
                "schema_id": schema_id(contract, predicates, shape),
                "shape": shape,
            }
        )
    return {
        "batch_legality": {
            "closed": True,
            "entries": [
                {"batch": "run_admission", "coordinate": "genesis", "records": ["run_admitted"]},
                {
                    "batch": "state_transition",
                    "coordinate": "exact_head",
                    "records": ["state_transition_committed", "run_closed?"],
                },
                {
                    "batch": "external_access_authorization",
                    "coordinate": "exact_head",
                    "records": ["external_access_authorized"],
                },
                {
                    "batch": "external_access_observation",
                    "coordinate": "exact_head",
                    "records": ["external_access_observed"],
                },
            ],
        },
        "canonicalization": {
            "algorithm": "sha256-jcs-v1",
            "domain_envelope_schema": "mfm.domain-separated-preimage.v1",
            "duplicate_keys": "reject",
            "floats": "forbidden",
            "integer_numbers": "unsigned_json_integers_only_where_schema_bounded",
            "max_canonical_json_bytes": "16777216",
            "object_key_order": "utf16_code_units",
            "string_normalization": "none",
        },
        "content_addressing": {
            "algorithm": "sha256-v1",
            "grammar": "content:sha256-v1:<64_lowercase_hex>",
            "input": "exact_retained_bytes",
            "semantic_identity_substitution": "forbidden",
        },
        "contract": "mfm.recoverability-annex.v1",
        "domains": [
            {
                "domain": "mfm.fact-content-identity.v1",
                "owner_predicates": ["P-FA-01"],
                "preimage_schema": "mfm.fact-content-identity-preimage.v1",
                "result_kind": "semantic_digest",
            },
            {
                "domain": "mfm.fact-logical-identity.v1",
                "owner_predicates": ["P-FA-01"],
                "preimage_schema": "mfm.fact-logical-identity-preimage.v1",
                "result_kind": "semantic_digest",
            },
            {
                "domain": "mfm.fact-query.v1",
                "owner_predicates": ["P-FA-01"],
                "preimage_schema": "mfm.fact-selection-request.v1",
                "result_kind": "semantic_digest",
            },
            {
                "domain": "mfm.run-id.v1",
                "owner_predicates": ["P-RH-01"],
                "preimage_schema": "mfm.run-id-preimage.v1",
                "result_kind": "run_id",
            },
            {
                "domain": "mfm.schema.v1",
                "owner_predicates": ["P-HB-01"],
                "preimage_schema": "mfm.schema-descriptor.v1",
                "result_kind": "schema_id",
            },
        ],
        "error_codes": {
            "codec": {
                "closed": True,
                "values": [
                    "duplicate_item",
                    "identity_construction",
                    "invalid_annex",
                    "invalid_canonical_json",
                    "invalid_order",
                    "invalid_value",
                    "missing_field",
                    "out_of_bounds",
                    "schema_mismatch",
                    "unknown_domain",
                    "unknown_field",
                    "unknown_schema",
                    "wrong_type",
                ],
            },
            "relational": {
                "closed": True,
                "values": [
                    "atomic_append_violation",
                    "configuration_predecessor_mismatch",
                    "old_contract_bytes",
                    "stale_head",
                    "wallet_fence_violation",
                ],
            },
        },
        "identity_encodings": {
            "access_attempt_id": "access-attempt:sha256-jcs-v1:<64_lowercase_hex>",
            "artifact_id": "artifact:sha256-jcs-v1:<64_lowercase_hex>",
            "content_digest": "content:sha256-v1:<64_lowercase_hex>",
            "failure_plan_id": "failure-plan:sha256-jcs-v1:<64_lowercase_hex>",
            "fragment_boundary_id": "fragment-boundary:sha256-jcs-v1:<64_lowercase_hex>",
            "occurrence_id": "occurrence:sha256-jcs-v1:<64_lowercase_hex>",
            "run_id": "run:sha256-jcs-v1:<64_lowercase_hex>",
            "schema_id": "schema:<name>:<positive-version>:sha256-jcs-v1:<64_lowercase_hex>",
            "semantic_call_id": "semantic-call:sha256-jcs-v1:<64_lowercase_hex>",
            "semantic_digest": "sha256-jcs-v1:<64_lowercase_hex>",
            "store_scope_id": "mfm.store_scope.v1:<32_lowercase_hex>",
            "tenant_scope_id": "mfm.tenant_scope.v1:<32_lowercase_hex>",
        },
        "limits": {
            "max_array_items": "1048576",
            "max_base64url_characters": "22369622",
            "max_canonical_json_bytes": "16777216",
            "max_object_entries": "1048576",
            "max_string_utf8_bytes": "16777216",
        },
        "logical_keys": {
            "closed": True,
            "entries": [
                {
                    "contract": "mfm.run-admission-logical-key.v1",
                    "fields": ["run_id"],
                    "owner_predicates": ["P-RH-01"],
                    "uniqueness": "one admission per run",
                },
                {
                    "contract": "mfm.state-transition-logical-key.v1",
                    "fields": ["occurrence_id"],
                    "owner_predicates": ["P-RH-01"],
                    "uniqueness": "one settlement per occurrence",
                },
                {
                    "contract": "mfm.access-authorization-logical-key.v1",
                    "fields": ["access_attempt_id"],
                    "owner_predicates": ["P-RH-01"],
                    "uniqueness": "one authorization per access attempt",
                },
                {
                    "contract": "mfm.access-observation-logical-key.v1",
                    "fields": ["access_attempt_id"],
                    "owner_predicates": ["P-RH-01"],
                    "uniqueness": "one observation per access attempt",
                },
                {
                    "contract": "mfm.run-closure-logical-key.v1",
                    "fields": ["run_id"],
                    "owner_predicates": ["P-RH-01"],
                    "uniqueness": "one closure per run",
                },
            ],
        },
        "schema_algebra": [
            "array",
            "boolean",
            "bounded_unsigned_integer",
            "canonical_decimal_u64",
            "literal",
            "nullable",
            "object",
            "optional_absent",
            "reference",
            "string",
            "tagged_union",
        ],
        "schemas": schemas,
        "transient_authority_types": {
            "closed": True,
            "serialization": "forbidden",
            "values": [
                "AdmissionCertificationRegistry",
                "AuthorizedAccess",
                "CandidateActivationPermit",
                "CommittedObservation",
                "ConfigurationHistoryWriter",
                "NewlyAppendedAuthorization",
                "OperationBuilder",
                "PendingObservation",
                "PhysicalBindingAuthorization",
                "ProgramRegistryBuilder",
                "QualifiedPhysicalBinding",
                "QualifiedProgramRegistry",
                "Runtime",
                "StateFrame",
                "RuntimeHistoryPort",
                "StoreHistoryAdapter",
                "WalletNonceAuthorityResource",
                "WalletNonceDomainActivationAttestation",
            ],
        },
    }


def digest_value(domain: str, value: Any, prefix: str = "") -> str:
    digest = hashlib.sha256(canonical({"domain": domain, "value": value})).hexdigest()
    return f"{prefix}sha256-jcs-v1:{digest}"


def content_digest(value: bytes) -> str:
    return f"content:sha256-v1:{hashlib.sha256(value).hexdigest()}"


def schema_descriptor(annex: dict[str, Any], contract: str) -> dict[str, Any]:
    return next(schema for schema in annex["schemas"] if schema["contract"] == contract)


def schema_vector(annex: dict[str, Any], contract: str, value: Any) -> dict[str, Any]:
    encoded = canonical(value)
    descriptor = schema_descriptor(annex, contract)
    return {
        "canonical_hex": encoded.hex(),
        "expected_content_digest": content_digest(encoded),
        "expected_schema_id": descriptor["schema_id"],
        "id": f"schema/{contract}/minimum",
        "input_hex": encoded.hex(),
        "kind": "schema_acceptance",
        "schema_contract": contract,
    }


def build_corpus(annex: dict[str, Any], annex_bytes: bytes) -> dict[str, Any]:
    zeros = "0" * 64
    ones = "1" * 64
    run_id = f"run:sha256-jcs-v1:{zeros}"
    semantic = f"sha256-jcs-v1:{zeros}"
    store_scope = f"mfm.store_scope.v1:{'0' * 32}"
    tenant_scope = f"mfm.tenant_scope.v1:{'1' * 32}"
    invocation = "00000000-0000-4000-8000-000000000000"
    content = {
        "content_digest": f"content:sha256-v1:{ones}",
        "schema_id": f"schema:mfm.test.fact:1:sha256-jcs-v1:{zeros}",
    }
    journal_head = {"commit_digest": semantic, "run_sequence": 1}
    occurrence_id = f"occurrence:sha256-jcs-v1:{'2' * 64}"
    query = {
        "canonical_predicate": {"sample": "value"},
        "fact_descriptor_ref": content,
        "limit": 1,
        "ordering": "ascending",
        "tie_break": "fact_identity_ascending",
    }
    request = {
        "admitted_source_manifest_ref": content,
        "completeness_mode": "complete_through_authorization_frontier",
        "producer_scope": "other_runs_in_tenant_scope",
        "queries": [query],
        "scan_bounds": {
            "maximum_facts": 16_000_000,
            "maximum_publications": 1_000_000,
            "maximum_response_bytes": 16 * 1024 * 1024,
            "maximum_retained_source_bytes": 512 * 1024 * 1024,
            "maximum_selected_results": 16_384,
        },
        "selector_contract_ref": {
            "content_digest": "content:sha256-v1:a9ac077123f465977c041695250809a21459c66dcbce4a16d84ca03f9d6a448c",
            "schema_id": "schema:mfm.prior-run-fact-selector-contract:1:sha256-jcs-v1:668cf10b8c62985601157a96c8fc5f53739e2e18837b9b40b8f18772925d1bac",
        },
        "version": "mfm.fact-selection-request.v1",
    }
    typed_value = {"contract_ref": content, "value_ref": content}
    fact_content_preimage = {
        "fact_descriptor_ref": content,
        "response_ref": typed_value,
        "subject_ref": typed_value,
    }
    transition_ref = {
        "ordinal": 0,
        "record_hash": semantic,
        "run_id": run_id,
        "run_sequence": 1,
    }
    fact_logical_preimage = {
        "emission_ordinal": 0,
        "fact_content_identity": digest_value(
            "mfm.fact-content-identity.v1", fact_content_preimage
        ),
        "transition_ref": transition_ref,
    }
    planning_ref = content
    positives = [
        schema_vector(
            annex,
            "mfm.structured-typed-value-ref.v1",
            typed_value,
        ),
        schema_vector(
            annex,
            "mfm.fact-content-identity-preimage.v1",
            fact_content_preimage,
        ),
        schema_vector(
            annex,
            "mfm.fact-logical-identity-preimage.v1",
            fact_logical_preimage,
        ),
        schema_vector(annex, "mfm.fact-selection-query.v1", query),
        schema_vector(annex, "mfm.fact-selection-request.v1", request),
        schema_vector(
            annex,
            "mfm.admit-run-request.v1",
            {
                "entry_point_id": "mfm.portfolio/snapshot@1",
                "input": {},
                "invocation_identity": invocation,
                "version": "mfm.admit-run-request.v1",
            },
        ),
        schema_vector(
            annex,
            "mfm.admit-run-response.v1",
            {
                "admission": "attached",
                "entry_point_id": "mfm.portfolio/snapshot@1",
                "entry_point_operation_id": "mfm.portfolio/snapshot",
                "invocation_identity": invocation,
                "planning_profile_ref": planning_ref,
                "run_id": run_id,
                "version": "mfm.admit-run-response.v1",
            },
        ),
        schema_vector(
            annex,
            "mfm.drive-response.v1",
            {"journal_head": journal_head, "kind": "advanced", "reason": None, "run_id": run_id},
        ),
        {
            **schema_vector(
                annex,
                "mfm.drive-response.v1",
                {
                    "journal_head": journal_head,
                    "kind": "waiting",
                    "reason": "retryable_evidence_gap",
                    "run_id": run_id,
                },
            ),
            "id": "schema/mfm.drive-response.v1/waiting",
        },
        {
            **schema_vector(
                annex,
                "mfm.drive-response.v1",
                {"journal_head": journal_head, "kind": "closed", "reason": None, "run_id": run_id},
            ),
            "id": "schema/mfm.drive-response.v1/closed",
        },
        schema_vector(
            annex,
            "mfm.public-run-view.v1",
            {
                "entry_point_operation_id": "mfm.portfolio/snapshot",
                "invocation_identity": invocation,
                "journal_head": journal_head,
                "outcome": None,
                "run_id": run_id,
                "semantic_head": {"kind": "genesis"},
                "status": "actionable",
                "tenant_scope_id": tenant_scope,
                "version": "mfm.public-run-view.v1",
            },
        ),
        schema_vector(
            annex,
            "mfm.public-error.v1",
            {
                "code": "AuthenticationRequired",
                "message": "Authentication is required",
            },
        ),
        {
            **schema_vector(
                annex,
                "mfm.public-error.v1",
                {
                    "code": "StructuredRuntimeCallbackFault",
                    "message": "A qualified Runtime callback failed",
                    "runtime_fault": {
                        "occurrence_id": occurrence_id,
                        "phase": "invoke_pure",
                        "pre_fault_head": journal_head,
                        "run_id": run_id,
                        "subject": {
                            "component_kind": "state",
                            "kind": "process",
                            "semantic_contract_ref": content,
                        },
                    },
                },
            ),
            "id": "schema/mfm.public-error.v1/runtime-process-fault",
        },
        {
            **schema_vector(
                annex,
                "mfm.error-response.v1",
                {
                    "error": {
                        "code": "ReplayVerificationFailed",
                        "message": "Recorded run evidence failed verification",
                        "runtime_fault": {
                            "occurrence_id": None,
                            "phase": "load_history",
                            "pre_fault_head": None,
                            "run_id": run_id,
                            "subject": {
                                "kind": "store",
                                "store_epoch": "7",
                                "store_scope_id": "mfm.store_scope.v1:" + "7" * 32,
                            },
                        },
                    },
                    "status": "error",
                },
            ),
            "id": "schema/mfm.error-response.v1/runtime-store-fault",
        },
    ]
    request_bytes = canonical(request)
    for domain, schema, preimage in [
        (
            "mfm.fact-content-identity.v1",
            "mfm.fact-content-identity-preimage.v1",
            fact_content_preimage,
        ),
        (
            "mfm.fact-logical-identity.v1",
            "mfm.fact-logical-identity-preimage.v1",
            fact_logical_preimage,
        ),
    ]:
        positives.append(
            {
                "domain": domain,
                "expected": {"value": digest_value(domain, preimage)},
                "id": f"domain/{domain}/minimum",
                "kind": "domain_identity",
                "preimage_schema": schema,
                "value_hex": canonical(preimage).hex(),
            }
        )
    positives.append(
        {
            "domain": "mfm.fact-query.v1",
            "expected": {"value": digest_value("mfm.fact-query.v1", request)},
            "id": "domain/mfm.fact-query.v1/minimum",
            "kind": "domain_identity",
            "preimage_schema": "mfm.fact-selection-request.v1",
            "value_hex": request_bytes.hex(),
        }
    )
    invalid_query = {**query, "limit": 0}
    hostile_public_error = {
        "code": "StructuredRuntimeCallbackFault",
        "message": "A qualified Runtime callback failed",
        "runtime_fault": {
            "occurrence_id": occurrence_id,
            "phase": "invoke_pure",
            "pre_fault_head": journal_head,
            "run_id": run_id,
            "subject": {
                "component_kind": "state",
                "implementation_ref": content,
                "kind": "process",
                "semantic_contract_ref": content,
            },
        },
    }
    negative = [
        {
            "expected_error": "out_of_bounds",
            "id": "codec/out-of-bounds/fact-query-limit",
            "input_hex": canonical(invalid_query).hex(),
            "kind": "schema_rejection",
            "schema_contract": "mfm.fact-selection-query.v1",
        },
        {
            "expected_error": "unknown_field",
            "id": "codec/unknown-field/public-runtime-implementation-ref",
            "input_hex": canonical(hostile_public_error).hex(),
            "kind": "schema_rejection",
            "schema_contract": "mfm.public-error.v1",
        },
    ]
    relation_bytes = canonical({"sample": "value"})
    relational = [
        {
            "content_digest": content_digest(relation_bytes),
            "id": "relation/semantic-vs-content",
            "kind": "content_identity",
            "value_hex": relation_bytes.hex(),
        }
    ]
    return {
        "artifacts": {
            "annex_bytes": len(annex_bytes),
            "annex_sha256": hashlib.sha256(annex_bytes).hexdigest(),
        },
        "contract": "mfm.recoverability-corpus.v1",
        "negative_vectors": negative,
        "positive_vectors": positives,
        "relational_vectors": relational,
    }


def main() -> None:
    annex = build_annex()
    annex_bytes = canonical(annex)
    corpus_bytes = canonical(build_corpus(annex, annex_bytes))
    (ROOT / "annex.json").write_bytes(annex_bytes)
    (ROOT / "corpus.json").write_bytes(corpus_bytes)


if __name__ == "__main__":
    main()
