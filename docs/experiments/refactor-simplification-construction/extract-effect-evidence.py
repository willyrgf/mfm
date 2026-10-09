#!/usr/bin/env python3
"""Extract public H4 facts while leaving solc initcode and creation wire temporary.

Run after the managed specimen task, whose compact stdout hashes each full dump:
  python3 extract-effect-evidence.py --dump-dir /owned-export/dumps \
      --output-dir evidence
Only the explicit public subset below is archived. This is not a lossless copy of
the temporary full consumer output or Store frame; the probe asserts exact byte
and Object equality, and pinned solc reproduces their artifact-containing parts.
"""
import argparse
import hashlib
import json
from pathlib import Path


def extract(dump_dir, output_dir):
    def load(name, optional=False):
        path = dump_dir / (name + ".json")
        if optional and not path.exists():
            return None
        return json.loads(path.read_text())

    expectations = {
        "artifact_sha256": "c7aed441e0afa86de84779ac168d27e4d8a29148b6834565d5930ef7c8ae855d",
        "artifact_length": 497,
        "requested": "42",
        "increment": "0",
        "deployment_gas": 2_000_000,
        "configuration_gas": 200_000,
        "priority_fee": "1000000000",
        "maximum_fee": "10000000000",
        "configuration_calldata_hex": "1eb25e0a000000000000000000000000000000000000000000000000000000000000002a",
        "getter_selector_hex": "3fa4f245",
        "getter_result_hex": "000000000000000000000000000000000000000000000000000000000000002a",
    }

    def run(kind):
        prepared_record = load(kind + "-command-frame")
        prepared = prepared_record["operation"]["effect_prepared"]
        settled_record = load(kind + "-settlement-frame")
        settled = settled_record["operation"]["effect_settled"]
        deployed = load(kind + "-complete-deployment")
        configured = load(kind + "-complete-configuration")
        assert prepared == settled["effect"]
        assert settled["evidence"] == deployed["deployment"]["original"]
        assert deployed["deployment"]["effect_id"] == prepared["effect_id"]
        assert deployed["deployment"]["command_ref"] == prepared["command"]["value_ref"]
        report = {
            "fixture_expectations": expectations,
            "program_ref": prepared_record["program_ref"],
            "semantic_command": {
                "effect_id": prepared["effect_id"],
                "position": prepared["call"]["position"],
                "value_ref": prepared["command"]["value_ref"],
            },
            "complete_native_original": settled["evidence"],
            "deployment_evidence": deployed["deployment"],
            "effective": deployed["effective"],
            "configuration_evidence": configured["configuration"],
            "observer_summary": load(kind + "-observer-summary", optional=True),
            "asserted_equalities": {
                "command_before_and_after_settlement": True,
                "stored_original_equals_output_original": True,
                "effect_id_equals_acknowledged_command_authority": True,
                "semantic_command_ref_equals_acknowledged_object": True,
            },
        }
        if kind == "scripted":
            wire = load("scripted-winning-wire")
            report["old_control"] = load("old-control-authority-lineage")
            refusal = load("scripted-partial-outcome")["failure"]["domain"]
            exposed = refusal["call"]["pure"]["input"]["canonical"]
            report["partial_refusal"] = {
                "original": refusal["original"],
                "effective": exposed["effective"],
                "deployment_evidence": exposed["deployment"],
                "requested": exposed["request"]["requested"],
                "increment": exposed["request"]["increment"],
                "configuration_preparations": 0,
                "typed_original": "AdditionOverflow",
                "typed_exposed_input": "DeployedContract",
            }
        else:
            wire = load("managed-authority-after-broadcast")
            report["independent_node_deployment_receipt"] = load("managed-node-deployment-receipt")
            report["independent_scalar_observation"] = load("managed-independent-scalar-observation")
            report["configuration_authority"] = load("managed-configuration-authority")
        raw = bytes.fromhex(wire.pop("raw_hex"))
        wire["raw_length"] = len(raw)
        wire["raw_sha256"] = hashlib.sha256(raw).hexdigest()
        report["retained_creation_winner"] = wire
        output_dir.mkdir(parents=True, exist_ok=True)
        (output_dir / (kind + "-observed.json")).write_text(json.dumps(report, indent=2) + "\n")

    run("scripted")
    if (dump_dir / "managed-command-frame.json").exists():
        run("managed")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dump-dir", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    arguments = parser.parse_args()
    extract(arguments.dump_dir, arguments.output_dir)
