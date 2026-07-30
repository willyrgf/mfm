#!/usr/bin/env python3
"""Generate the frozen recoverability-v3 annex and conformance corpus.

The v1 and v2 directories are immutable inputs. This script performs only the
reviewed v2-to-v3 closed-schema cutover and refuses to run when either archive
does not have its frozen digest.
"""

from __future__ import annotations

import copy
import hashlib
import json
import pathlib
import re
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[1]
V1 = ROOT / "contracts/recoverability/v1"
V2 = ROOT / "contracts/recoverability/v2"
V3 = ROOT / "contracts/recoverability/v3"

FROZEN = {
    V1 / "README.md": "bdec7624585a046407b9aaac6c36868050af2c52fa4f815cb05ebafecc5a8ed0",
    V1 / "annex.json": "a3fb5cf2e0486a1a1e906c2fd93b10b3f0f52c5a785b163b6cc758ff39a4defe",
    V1 / "corpus.json": "8d10c1a05820a18781a4864fb2d47248d6de41689db5fe0ed4800dd8b0742f82",
    V2 / "README.md": "2adb381aca75c126746fdca28b2b84105dee5144c4e49c35cef6e899d9fc5217",
    V2 / "annex.json": "bf1065f32a8249f69b9a82f19be2a221d1db5b683fa439ec6f701c42666b3ff8",
    V2 / "corpus.json": "a2b249e054e7c6e85edfd91a6f3c6c9fc9588f65a9f3dfbfe0eac08f9f0be7a9",
}

CONSUMERS = [
    "mfm-canonical",
    "mfm-ids",
    "mfm-certify",
    "mfm-executor",
    "mfm-store-memory",
    "mfm-storage-postgres",
    "mfm-runtime",
    "mfm-replay",
    "mfm-trace-export",
]

RUN_SUCCESSORS = [
    "observation-outcome",
    "external-access-observed",
    "run-journal-record",
    "record-hash-preimage",
    "candidate-record-envelope",
    "commit-candidate-preimage",
    "legal-commit-batch",
    "access-audit-entry",
    "pending-effect",
    "active-run-view",
    "public-run-view",
    "portable-run-export-frame",
    "portable-run-export-stream",
]

EXECUTOR_SUCCESSORS = [
    "executor-reference-failure-code",
    "executor-reference-safe-failure",
    "executor-reference-queue-result",
    "executor-attempt-outcome",
    "executor-delivery-attempt-observed",
    "executor-reference-terminal-proof",
    "executor-terminal-tombstone",
    "executor-evidence-record",
    "executor-delivery-frontier",
    "executor-frontier-preimage",
    "executor-record-preimage",
]

REPLACEMENTS = {
    **{f"mfm.{name}.v1": f"mfm.{name}.v2" for name in RUN_SUCCESSORS},
    **{f"mfm.{name}.v1": f"mfm.{name}.v2" for name in EXECUTOR_SUCCESSORS},
    "mfm.journal-record.v1": "mfm.journal-record.v2",
    "mfm.journal-candidate.v1": "mfm.journal-candidate.v2",
    "mfm.executor-frontier.v1": "mfm.executor-frontier.v2",
    "mfm.executor-record.v1": "mfm.executor-record.v2",
    "application/vnd.mfm.run-export-stream.v1+json-seq":
        "application/vnd.mfm.run-export-stream.v2+json-seq",
    "mfm.[a-z0-9._-]+.v1": "mfm.[a-z0-9._-]+.v[1-9][0-9]*",
    "schema:<name>:1:sha256-jcs-v1:[0-9a-f]{64}":
        "schema:<name>:<positive-version>:sha256-jcs-v1:[0-9a-f]{64}",
    "schema:<name>:1:sha256-jcs-v1:<64_lowercase_hex>":
        "schema:<name>:<positive-version>:sha256-jcs-v1:<64_lowercase_hex>",
}

CONTRACT_RE = re.compile(r"^(?P<name>mfm\.[a-z0-9._-]+)\.v(?P<version>[1-9][0-9]*)$")


def canonical(value: Any) -> bytes:
    return json.dumps(
        value, ensure_ascii=False, sort_keys=True, separators=(",", ":")
    ).encode("utf-8")


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def require_frozen_archives() -> None:
    for path, expected in FROZEN.items():
        actual = digest(path.read_bytes())
        if actual != expected:
            raise SystemExit(f"frozen archive changed: {path}: {actual}")


def replace_text(value: str, extra: dict[str, str] | None = None) -> str:
    replacements = REPLACEMENTS if extra is None else {**REPLACEMENTS, **extra}
    for old, new in sorted(replacements.items(), key=lambda item: -len(item[0])):
        value = value.replace(old, new)
    return value


def deep_replace(value: Any, extra: dict[str, str] | None = None) -> Any:
    if isinstance(value, str):
        return replace_text(value, extra)
    if isinstance(value, list):
        return [deep_replace(item, extra) for item in value]
    if isinstance(value, dict):
        return {key: deep_replace(item, extra) for key, item in value.items()}
    return value


def string_schema(contract: str, values: list[str], owners: list[str]) -> dict[str, Any]:
    return {
        "contract": contract,
        "owner_predicates": owners,
        "schema_id": "",
        "shape": {
            "kind": "string",
            "max_length": max(map(len, values)),
            "min_length": min(map(len, values)),
            "values": values,
        },
    }


def non_domain_schemas() -> list[dict[str, Any]]:
    return [
        string_schema(
            "mfm.non-domain-entry-status.v1",
            ["proven_not_entered", "may_have_entered"],
            ["P-AR-01"],
        ),
        string_schema(
            "mfm.non-domain-disposition.v1",
            ["retryable_operational", "integrity_blocked"],
            ["P-AR-01"],
        ),
        string_schema(
            "mfm.non-domain-failure-code.v1",
            [
                "adapter_contract_violation",
                "result_encoding_failure",
                "fact_store_unavailable",
                "fact_history_invalid",
                "executor_store_unavailable",
                "executor_contention",
                "executor_history_invalid",
                "executor_capacity_exhausted",
            ],
            ["P-AR-01"],
        ),
        {
            "contract": "mfm.non-domain-failure.v1",
            "owner_predicates": ["P-AR-01"],
            "schema_id": "",
            "shape": {
                "fields": [
                    {
                        "name": "entry_status",
                        "presence": "required",
                        "type": {
                            "contract": "mfm.non-domain-entry-status.v1",
                            "kind": "reference",
                        },
                    },
                    {
                        "name": "disposition",
                        "presence": "required",
                        "type": {
                            "contract": "mfm.non-domain-disposition.v1",
                            "kind": "reference",
                        },
                    },
                    {
                        "name": "code",
                        "presence": "required",
                        "type": {
                            "contract": "mfm.non-domain-failure-code.v1",
                            "kind": "reference",
                        },
                    },
                ],
                "invariants": [
                    "adapter_contract_violation and result_encoding_failure accept either entry status and require integrity_blocked",
                    "fact_store_unavailable is fact-only, may_have_entered, and retryable_operational",
                    "fact_history_invalid is fact-only, may_have_entered, and integrity_blocked",
                    "executor_store_unavailable and executor_contention are ensure-only, accept either entry status, and require retryable_operational",
                    "executor_history_invalid is ensure-only, accepts either entry status, and requires integrity_blocked",
                    "executor_capacity_exhausted is ensure-only, proven_not_entered, and integrity_blocked",
                    "the contextual read, fact, ensure, or executor-target layer is validated by its owning fold",
                ],
                "kind": "object",
                "unknown_fields": "reject",
            },
        },
    ]


def descriptor(annex: dict[str, Any], contract: str) -> dict[str, Any]:
    return next(item for item in annex["schemas"] if item["contract"] == contract)


def add_non_domain_variant(annex: dict[str, Any]) -> None:
    variant = {
        "fields": [
            {
                "name": "non_domain_failure",
                "presence": "required",
                "type": {
                    "contract": "mfm.non-domain-failure.v1",
                    "kind": "reference",
                },
            }
        ],
        "tag": "non_domain_failure",
    }
    observation = descriptor(annex, "mfm.observation-outcome.v2")["shape"]
    observation["variants"].append(copy.deepcopy(variant))
    observation["invariants"] = [
        "non_domain_failure is audit-only and cannot satisfy a state transition"
    ]
    attempt = descriptor(annex, "mfm.executor-attempt-outcome.v2")["shape"]
    attempt["variants"].append(variant)
    attempt["invariants"] = [
        "non_domain_failure is audit-only and cannot satisfy terminal delivery proof"
    ]


def update_access_audit(annex: dict[str, Any]) -> None:
    shape = descriptor(annex, "mfm.access-audit-entry.v2")["shape"]
    status = next(field for field in shape["fields"] if field["name"] == "status")
    status["type"]["values"].append("non_domain_failure")
    status["type"]["min_length"] = min(map(len, status["type"]["values"]))
    status["type"]["max_length"] = max(map(len, status["type"]["values"]))
    failure_index = next(
        index for index, field in enumerate(shape["fields"]) if field["name"] == "failure"
    )
    shape["fields"].insert(
        failure_index + 1,
        {
            "name": "non_domain_failure",
            "presence": "required",
            "type": {
                "kind": "nullable",
                "value": {
                    "contract": "mfm.non-domain-failure.v1",
                    "kind": "reference",
                },
            },
        },
    )
    shape["invariants"].extend(
        [
            "non_domain_failure is non-null exactly when status is non_domain_failure",
            "failure and non_domain_failure are mutually exclusive",
        ]
    )


def update_pending_effect(annex: dict[str, Any]) -> None:
    shape = descriptor(annex, "mfm.pending-effect.v2")["shape"]
    status = next(
        field for field in shape["fields"] if field["name"] == "executor_status"
    )["type"]
    status["values"].extend(
        ["non_domain_retryable", "non_domain_integrity_blocked"]
    )
    status["min_length"] = min(map(len, status["values"]))
    status["max_length"] = max(map(len, status["values"]))


def update_executor_failure(annex: dict[str, Any]) -> None:
    code = descriptor(annex, "mfm.executor-reference-failure-code.v2")["shape"]
    code["values"].append("result_unrepresentable")
    code["min_length"] = min(map(len, code["values"]))
    code["max_length"] = max(map(len, code["values"]))
    safe = descriptor(annex, "mfm.executor-reference-safe-failure.v2")["shape"]
    safe["invariants"].append(
        "result_unrepresentable is indeterminate/unrepresentable_response/boundary_observation"
    )


def update_evidence_bounds(annex: dict[str, Any]) -> None:
    shape = descriptor(annex, "mfm.evidence-bounds.v1")["shape"]
    max_retained_index = next(
        index
        for index, field in enumerate(shape["fields"])
        if field["name"] == "max_retained_bytes"
    )
    shape["fields"].insert(
        max_retained_index + 1,
        {
            "name": "max_completion_record_bytes",
            "presence": "required",
            "type": {
                "contract": "mfm.primitive-decimal_u64.v1",
                "kind": "reference",
            },
        },
    )
    shape["invariants"] = [
        "when max_attempts is zero max_completion_record_bytes, completion_reserve_records, and completion_reserve_bytes are zero",
        "when max_attempts is nonzero max_completion_record_bytes is positive and at most completion_reserve_bytes, completion_reserve_bytes is at most max_retained_bytes, and completion_reserve_records is exactly two",
        "every observed-attempt or terminal-tombstone one-record frontier content closure is at most max_completion_record_bytes",
        "capacity covers the actual deduplicated canonical retained closure plus one completion_reserve_bytes debt for every unmatched authorization and one shared terminal tombstone while absent",
        "max_attempts is at most 64 for the retained reference contract",
    ]


def schema_identity(schema: dict[str, Any]) -> str:
    match = CONTRACT_RE.fullmatch(schema["contract"])
    if match is None:
        raise ValueError(f"invalid schema contract: {schema['contract']}")
    preimage = {key: value for key, value in schema.items() if key != "schema_id"}
    envelope = {"domain": "mfm.schema.v1", "value": preimage}
    return (
        f"schema:{match.group('name')}:{match.group('version')}:sha256-jcs-v1:"
        f"{digest(canonical(envelope))}"
    )


def build_annex(source: dict[str, Any]) -> tuple[dict[str, Any], dict[str, str]]:
    old_ids = {item["contract"]: item["schema_id"] for item in source["schemas"]}
    annex = deep_replace(copy.deepcopy(source))
    annex["contract"] = "mfm.recoverability-annex.v3"
    annex["identity_encodings"]["schema_id"] = (
        "schema:<name>:<positive-version>:sha256-jcs-v1:<64_lowercase_hex>"
    )
    annex["error_codes"]["relational"]["values"].append(
        "non_domain_failure_relation"
    )
    annex["error_codes"]["relational"]["values"].sort()
    annex["schemas"].extend(non_domain_schemas())
    add_non_domain_variant(annex)
    update_access_audit(annex)
    update_pending_effect(annex)
    update_executor_failure(annex)
    update_evidence_bounds(annex)
    transient = [
        authority
        for authority in annex["transient_authority_types"]["values"]
        if authority != "TargetOperationReceipt"
    ]
    transient.extend(
        [
            "AuthorizedEvmWalletTarget",
            "PendingObservation",
            "PreparedAccess",
            "PreparedEvmWalletTarget",
            "TargetEntryAuthority",
        ]
    )
    annex["transient_authority_types"]["values"] = sorted(set(transient))
    annex["schemas"].sort(key=lambda item: item["contract"])
    for schema in annex["schemas"]:
        schema["schema_id"] = schema_identity(schema)
    new_by_old_contract = {
        old_contract: next(
            item["schema_id"]
            for item in annex["schemas"]
            if item["contract"] == replace_text(old_contract)
        )
        for old_contract in old_ids
    }
    id_map = {
        old_ids[contract]: new_by_old_contract[contract] for contract in old_ids
    }
    return annex, id_map


def transform_json_bytes(raw: bytes, id_map: dict[str, str]) -> bytes:
    if raw.startswith(b"\x1e"):
        records = []
        for record in raw.split(b"\x1e")[1:]:
            if not record.endswith(b"\n"):
                return raw
            records.append(
                b"\x1e" + transform_json_bytes(record[:-1], id_map) + b"\n"
            )
        return b"".join(records)
    try:
        value = json.loads(raw)
    except (UnicodeDecodeError, json.JSONDecodeError):
        return raw
    value = deep_replace(value, id_map)
    add_required_max_completion_record_bytes(value)
    add_required_non_domain_null(value)
    return canonical(value)


def add_required_max_completion_record_bytes(value: Any) -> None:
    if isinstance(value, list):
        for item in value:
            add_required_max_completion_record_bytes(item)
        return
    if not isinstance(value, dict):
        return
    legacy_fields = {
        "completion_reserve_bytes",
        "completion_reserve_records",
        "max_attempts",
        "max_records",
        "max_retained_bytes",
    }
    if legacy_fields.issubset(value):
        value.setdefault(
            "max_completion_record_bytes",
            value["completion_reserve_bytes"],
        )
    for item in value.values():
        add_required_max_completion_record_bytes(item)


def add_required_non_domain_null(value: Any) -> None:
    if isinstance(value, list):
        for item in value:
            add_required_non_domain_null(item)
        return
    if not isinstance(value, dict):
        return
    if value.get("version") == "mfm.access-audit-entry.v2":
        value.setdefault("non_domain_failure", None)
    for item in value.values():
        add_required_non_domain_null(item)


def transform_hex_fields(value: Any, id_map: dict[str, str]) -> None:
    if isinstance(value, list):
        for item in value:
            transform_hex_fields(item, id_map)
        return
    if not isinstance(value, dict):
        return
    for key, item in list(value.items()):
        if key.endswith("_hex") and isinstance(item, str):
            try:
                raw = bytes.fromhex(item)
            except ValueError:
                continue
            value[key] = transform_json_bytes(raw, id_map).hex()
        else:
            transform_hex_fields(item, id_map)


def update_schema_vectors(
    corpus: dict[str, Any], annex: dict[str, Any], id_map: dict[str, str]
) -> None:
    schemas = {item["contract"]: item for item in annex["schemas"]}
    for vector in corpus["positive_vectors"]:
        if vector["kind"] != "schema_acceptance":
            continue
        contract = vector["schema_contract"]
        raw = bytes.fromhex(vector["canonical_hex"])
        vector["input_hex"] = raw.hex()
        vector["expected_content_digest"] = f"content:sha256-v1:{digest(raw)}"
        vector["expected_schema_id"] = schemas[contract]["schema_id"]

    samples = [
        (
            "mfm.non-domain-entry-status.v1",
            "proven_not_entered",
            "schema/mfm.non-domain-entry-status.v1/proven",
        ),
        (
            "mfm.non-domain-disposition.v1",
            "integrity_blocked",
            "schema/mfm.non-domain-disposition.v1/integrity",
        ),
        (
            "mfm.non-domain-failure-code.v1",
            "adapter_contract_violation",
            "schema/mfm.non-domain-failure-code.v1/adapter",
        ),
        (
            "mfm.non-domain-failure.v1",
            {
                "code": "adapter_contract_violation",
                "disposition": "integrity_blocked",
                "entry_status": "proven_not_entered",
            },
            "schema/mfm.non-domain-failure.v1/adapter",
        ),
        (
            "mfm.observation-outcome.v2",
            {
                "kind": "non_domain_failure",
                "non_domain_failure": {
                    "code": "adapter_contract_violation",
                    "disposition": "integrity_blocked",
                    "entry_status": "proven_not_entered",
                },
            },
            "schema/mfm.observation-outcome.v2/non-domain",
        ),
        (
            "mfm.executor-attempt-outcome.v2",
            {
                "kind": "non_domain_failure",
                "non_domain_failure": {
                    "code": "adapter_contract_violation",
                    "disposition": "integrity_blocked",
                    "entry_status": "may_have_entered",
                },
            },
            "schema/mfm.executor-attempt-outcome.v2/non-domain",
        ),
    ]
    for contract, sample, vector_id in samples:
        raw = canonical(sample)
        corpus["positive_vectors"].append(
            {
                "canonical_hex": raw.hex(),
                "consumer_coverage": CONSUMERS,
                "expected_content_digest": f"content:sha256-v1:{digest(raw)}",
                "expected_schema_id": schemas[contract]["schema_id"],
                "id": vector_id,
                "input_hex": raw.hex(),
                "kind": "schema_acceptance",
                "schema_contract": contract,
            }
        )


def update_evidence_bounds_vectors(corpus: dict[str, Any]) -> None:
    for collection in [
        corpus["positive_vectors"],
        corpus["negative_vectors"],
        corpus["relational_vectors"],
    ]:
        for vector in collection:
            if (
                vector.get("schema_contract") != "mfm.evidence-bounds.v1"
                and vector.get("target") != "mfm.evidence-bounds.v1"
            ):
                continue
            raw = json.loads(bytes.fromhex(vector["input_hex"]))
            raw["max_completion_record_bytes"] = (
                "0"
                if vector["id"] == "relational/reserve_exhausted"
                else "1024"
            )
            encoded = canonical(raw)
            vector["input_hex"] = encoded.hex()
            if "canonical_hex" in vector:
                vector["canonical_hex"] = encoded.hex()

    zero = canonical(
        {
            "completion_reserve_bytes": "0",
            "completion_reserve_records": 0,
            "max_attempts": 0,
            "max_completion_record_bytes": "0",
            "max_records": 1,
            "max_retained_bytes": "4096",
        }
    )
    corpus["positive_vectors"].append(
        {
            "canonical_hex": zero.hex(),
            "consumer_coverage": CONSUMERS,
            "expected_content_digest": "",
            "expected_schema_id": "",
            "id": "schema/mfm.evidence-bounds.v1/no-attempts",
            "input_hex": zero.hex(),
            "kind": "schema_acceptance",
            "schema_contract": "mfm.evidence-bounds.v1",
        }
    )

    legacy = canonical(
        {
            "completion_reserve_bytes": "1024",
            "completion_reserve_records": 2,
            "max_attempts": 1,
            "max_records": 4,
            "max_retained_bytes": "4096",
        }
    )
    corpus["negative_vectors"].append(
        {
            "consumer_coverage": CONSUMERS,
            "error_class": "codec",
            "expected_error": "missing_field",
            "id": "codec/missing-field/evidence-max-completion-record-bytes",
            "input_hex": legacy.hex(),
            "kind": "rejection",
            "rule": "legacy evidence bounds without the completion-record maximum are rejected",
            "target": "mfm.evidence-bounds.v1",
        }
    )


def update_domain_vectors(corpus: dict[str, Any]) -> None:
    for vector in corpus["positive_vectors"]:
        if vector["kind"] != "domain_identity":
            continue
        value = json.loads(bytes.fromhex(vector["value_hex"]))
        envelope = {"domain": vector["domain"], "value": value}
        encoded = canonical(envelope)
        computed = digest(encoded)
        vector["envelope_hex"] = encoded.hex()
        vector["expected"]["digest_hex"] = computed
        if vector["expected"]["kind"] == "schema_id":
            match = CONTRACT_RE.fullmatch(value["contract"])
            if match is None:
                raise ValueError(value["contract"])
            vector["expected"]["value"] = (
                f"schema:{match.group('name')}:{match.group('version')}:"
                f"sha256-jcs-v1:{computed}"
            )
        else:
            prefix = vector["expected"]["value"].rsplit(":", 1)[0]
            vector["expected"]["value"] = f"{prefix}:{computed}"


def update_schema_identity_vectors(corpus: dict[str, Any]) -> None:
    for vector in corpus["positive_vectors"] + corpus["relational_vectors"]:
        if vector["kind"] != "schema_identity":
            continue
        descriptor = json.loads(bytes.fromhex(vector["descriptor_preimage_hex"]))
        match = CONTRACT_RE.fullmatch(descriptor["contract"])
        if match is None:
            raise ValueError(descriptor["contract"])
        computed = digest(
            canonical({"domain": "mfm.schema.v1", "value": descriptor})
        )
        vector["schema_id"] = (
            f"schema:{match.group('name')}:{match.group('version')}:sha256-jcs-v1:"
            f"{computed}"
        )
        if isinstance(vector["expected"], dict):
            vector["expected"]["digest_hex"] = computed
            vector["expected"]["value"] = vector["schema_id"]


def update_raw_digest_vectors(corpus: dict[str, Any]) -> None:
    for vector in corpus["positive_vectors"] + corpus["relational_vectors"]:
        kind = vector["kind"]
        if kind == "content_addressing":
            vector["expected_content_digest"] = (
                f"content:sha256-v1:{digest(bytes.fromhex(vector['input_hex']))}"
            )
        elif kind == "content_identity_separation":
            for name in ["base", "nul_suffixed", "prefixed"]:
                vector[f"{name}_digest"] = (
                    f"content:sha256-v1:{digest(bytes.fromhex(vector[f'{name}_hex']))}"
                )
        elif kind == "request_identity":
            for input_name, output_name in [
                ("preimage_hex", "request_digest"),
                ("changed_preimage_hex", "changed_request_digest"),
            ]:
                value = json.loads(bytes.fromhex(vector[input_name]))
                computed = digest(
                    canonical({"domain": "mfm.request.v1", "value": value})
                )
                vector[output_name] = f"sha256-jcs-v1:{computed}"
        elif kind == "export_identity":
            vector["expected_content_digest"] = (
                f"content:sha256-v1:{digest(bytes.fromhex(vector['stream_hex']))}"
            )


def update_identity_separation(corpus: dict[str, Any]) -> None:
    for collection in [corpus["positive_vectors"], corpus["relational_vectors"]]:
        for vector in collection:
            if vector["kind"] != "identity_separation":
                continue
            if "semantic_envelope_hex" in vector:
                computed = digest(bytes.fromhex(vector["semantic_envelope_hex"]))
                vector["semantic_digest"] = f"sha256-jcs-v1:{computed}"
                continue
            for side in ["first", "second"]:
                encoded = bytes.fromhex(vector[f"{side}_envelope_hex"])
                computed = digest(encoded)
                vector[f"{side}_digest_hex"] = computed


def update_relational_acceptance(corpus: dict[str, Any]) -> None:
    for vector in corpus["relational_vectors"]:
        if vector["kind"] != "relational_acceptance":
            continue
        observation = bytes.fromhex(vector["observation_hex"])
        proof = json.loads(bytes.fromhex(vector["proof_hex"]))
        proof["returned_observation_ref"]["content_digest"] = (
            f"content:sha256-v1:{digest(observation)}"
        )
        proof_bytes = canonical(proof)
        vector["proof_hex"] = proof_bytes.hex()
        tombstone = json.loads(bytes.fromhex(vector["tombstone_hex"]))
        tombstone["terminal_proof_ref"]["content_digest"] = (
            f"content:sha256-v1:{digest(proof_bytes)}"
        )
        vector["tombstone_hex"] = canonical(tombstone).hex()


def add_relation_rejections(corpus: dict[str, Any]) -> None:
    codes = [
        "adapter_contract_violation",
        "result_encoding_failure",
        "fact_store_unavailable",
        "fact_history_invalid",
        "executor_store_unavailable",
        "executor_contention",
        "executor_history_invalid",
        "executor_capacity_exhausted",
    ]
    statuses = ["proven_not_entered", "may_have_entered"]
    dispositions = ["retryable_operational", "integrity_blocked"]
    valid = {
        ("adapter_contract_violation", status, "integrity_blocked")
        for status in statuses
    } | {
        ("result_encoding_failure", status, "integrity_blocked")
        for status in statuses
    } | {
        ("fact_store_unavailable", "may_have_entered", "retryable_operational"),
        ("fact_history_invalid", "may_have_entered", "integrity_blocked"),
        ("executor_capacity_exhausted", "proven_not_entered", "integrity_blocked"),
    } | {
        (code, status, "retryable_operational")
        for code in ["executor_store_unavailable", "executor_contention"]
        for status in statuses
    } | {
        ("executor_history_invalid", status, "integrity_blocked")
        for status in statuses
    }
    invalid = [
        {
            "code": code,
            "disposition": disposition,
            "entry_status": status,
        }
        for code in codes
        for status in statuses
        for disposition in dispositions
        if (code, status, disposition) not in valid
    ]
    raw = canonical(
        {"case": "non_domain_failure_relation", "invalid": invalid}
    )
    corpus["negative_vectors"].append(
        {
            "consumer_coverage": CONSUMERS,
            "error_class": "relational",
            "expected_error": "non_domain_failure_relation",
            "id": "relational/non_domain_failure_relation",
            "input_hex": raw.hex(),
            "kind": "rejection",
            "rule": "every non-domain code has one closed entry-status and disposition relation",
            "target": "mfm.non-domain-failure.v1",
        }
    )


def update_coverage(corpus: dict[str, Any], annex: dict[str, Any]) -> None:
    coverage = corpus["coverage"]
    coverage["mandatory_consumers"] = CONSUMERS
    coverage["schema_count"] = len(annex["schemas"])
    coverage["domain_count"] = len(annex["domains"])
    coverage["schema_invariant_clause_count"] = sum(
        len(schema["shape"].get("invariants", [])) for schema in annex["schemas"]
    )
    coverage["positive_vector_count"] = len(corpus["positive_vectors"])
    coverage["negative_vector_count"] = len(corpus["negative_vectors"])
    coverage["relational_vector_count"] = len(corpus["relational_vectors"])
    coverage["schema_vector_count"] = sum(
        vector["kind"] == "schema_acceptance"
        for vector in corpus["positive_vectors"]
    )
    coverage["domain_vector_count"] = sum(
        vector["kind"] == "domain_identity" for vector in corpus["positive_vectors"]
    )
    coverage["batch_vector_count"] = sum(
        vector["kind"] == "legal_batch" for vector in corpus["positive_vectors"]
    )
    coverage["relational_error_count"] = sum(
        vector.get("error_class") == "relational"
        for vector in corpus["negative_vectors"]
    )


def build_corpus(
    source: dict[str, Any], annex: dict[str, Any], id_map: dict[str, str]
) -> dict[str, Any]:
    corpus = deep_replace(copy.deepcopy(source), id_map)
    corpus["contract"] = "mfm.recoverability-corpus.v3"
    corpus["annex_content_digest"] = (
        f"content:sha256-v1:{digest(canonical(annex))}"
    )
    transform_hex_fields(corpus, id_map)
    update_evidence_bounds_vectors(corpus)
    source_negative = {
        vector["id"]: vector for vector in source["negative_vectors"]
    }
    for vector in corpus["negative_vectors"]:
        if vector.get("expected_error") == "invalid_canonical_json":
            vector["input_hex"] = source_negative[vector["id"]]["input_hex"]
    update_schema_vectors(corpus, annex, id_map)
    update_domain_vectors(corpus)
    update_schema_identity_vectors(corpus)
    update_raw_digest_vectors(corpus)
    update_identity_separation(corpus)
    update_relational_acceptance(corpus)
    add_relation_rejections(corpus)
    corpus["positive_vectors"].sort(key=lambda vector: vector["id"])
    corpus["negative_vectors"].sort(key=lambda vector: vector["id"])
    corpus["relational_vectors"].sort(key=lambda vector: vector["id"])
    update_coverage(corpus, annex)
    return corpus


def README(annex_bytes: bytes, corpus_bytes: bytes, annex: Any, corpus: Any) -> str:
    stream = descriptor(annex, "mfm.portable-run-export-stream.v2")
    frame = descriptor(annex, "mfm.portable-run-export-frame.v2")
    invariants = sum(
        len(schema["shape"].get("invariants", [])) for schema in annex["schemas"]
    )
    coverage = corpus["coverage"]
    total = (
        len(corpus["positive_vectors"])
        + len(corpus["negative_vectors"])
        + len(corpus["relational_vectors"])
    )
    return f"""# Recoverability v3 authority

`annex.json` and `corpus.json` are the sole current production recoverability
contract. Both files are exact, float-free canonical JSON bytes. The complete
v1 and v2 directories remain byte-identical archival and hostile-input
references; production consumers must not load either archive.

This is a destructive lineage cutover. There is no compatibility parser,
database migration, checkpoint upgrade, dual reader, or fallback.

Portable run transfer uses `mfm.portable-run-export-stream.v2` with media type
`application/vnd.mfm.run-export-stream.v2+json-seq`. Its sole integrity value
is the external raw SHA-256 over every RS, canonical frame byte, and LF.

## Frozen artifact metadata

```text
annex_bytes: {len(annex_bytes)}
annex_sha256: {digest(annex_bytes)}
annex_schema_count: {len(annex["schemas"])}
annex_invariant_clause_count: {invariants}
corpus_bytes: {len(corpus_bytes)}
corpus_sha256: {digest(corpus_bytes)}
corpus_positive_case_count: {len(corpus["positive_vectors"])}
corpus_negative_case_count: {len(corpus["negative_vectors"])}
corpus_relational_case_count: {len(corpus["relational_vectors"])}
corpus_total_case_count: {total}
corpus_schema_acceptance_case_count: {coverage["schema_vector_count"]}
corpus_codec_rejection_case_count: {coverage["codec_vector_count"]}
corpus_relational_rejection_case_count: {coverage["relational_error_count"]}
```

All {total} vectors are mandatory for each of the nine consumers named by
`corpus.json`.

## Portable stream registry

```text
stream_contract: mfm.portable-run-export-stream.v2
stream_schema_id: {stream["schema_id"]}
frame_contract: mfm.portable-run-export-frame.v2
frame_schema_id: {frame["schema_id"]}
```
"""


def main() -> None:
    require_frozen_archives()
    source_annex = json.loads((V2 / "annex.json").read_bytes())
    source_corpus = json.loads((V2 / "corpus.json").read_bytes())
    annex, id_map = build_annex(source_annex)
    corpus = build_corpus(source_corpus, annex, id_map)
    annex_bytes = canonical(annex)
    corpus_bytes = canonical(corpus)
    V3.mkdir(parents=True, exist_ok=True)
    (V3 / "annex.json").write_bytes(annex_bytes)
    (V3 / "corpus.json").write_bytes(corpus_bytes)
    (V3 / "README.md").write_text(
        README(annex_bytes, corpus_bytes, annex, corpus), encoding="utf-8"
    )
    require_frozen_archives()


if __name__ == "__main__":
    main()
