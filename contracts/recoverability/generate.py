#!/usr/bin/env python3
"""Generate the one current structured recoverability annex and corpus."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent / "v1"

# These values are the sole source for the generated Rust budget module and
# the public recoverability annex. Runtime codecs import the generated module;
# they must not repeat these numbers in production code.
LIMITS: dict[str, tuple[str, int, str]] = {
    "max_array_items": ("MAX_ARRAY_ITEMS", 1_048_576, "usize"),
    "max_base64url_characters": ("MAX_BASE64URL_CHARACTERS", 22_369_622, "usize"),
    "max_canonical_json_bytes": ("MAX_CANONICAL_JSON_BYTES", 32 * 1024 * 1024, "usize"),
    "max_canonical_json_depth": ("MAX_CANONICAL_JSON_DEPTH", 64, "usize"),
    "max_object_entries": ("MAX_OBJECT_ENTRIES", 1_048_576, "usize"),
    "max_string_utf8_bytes": ("MAX_STRING_UTF8_BYTES", 16_777_216, "usize"),
    "max_stored_frame_bytes": ("MAX_STORED_FRAME_BYTES", 32 * 1024 * 1024, "usize"),
    "max_batch_objects": ("MAX_BATCH_OBJECTS", 65_536, "usize"),
    "max_batch_records": ("MAX_BATCH_RECORDS", 65_536, "usize"),
    "max_portable_export_bytes": ("MAX_PORTABLE_EXPORT_BYTES", 16 * 1024 * 1024, "u64"),
    "max_portable_frame_bytes": ("MAX_PORTABLE_FRAME_BYTES", 1_048_576, "usize"),
    "max_portable_frames": ("MAX_PORTABLE_FRAMES", 1_048_577, "usize"),
    "max_portable_batches": ("MAX_PORTABLE_BATCHES", 1_048_576, "usize"),
    "max_portable_objects": ("MAX_PORTABLE_OBJECTS", 1_048_576, "usize"),
    "max_portable_source_runs": ("MAX_PORTABLE_SOURCE_RUNS", 4_096, "usize"),
    "max_portable_fact_routes": ("MAX_PORTABLE_FACT_ROUTES", 1_048_576, "usize"),
    "max_prior_run_source_rules": ("MAX_PRIOR_RUN_SOURCE_RULES", 1_024, "usize"),
    "max_prior_run_source_programs_per_rule": (
        "MAX_PRIOR_RUN_SOURCE_PROGRAMS_PER_RULE",
        4_096,
        "usize",
    ),
    "max_prior_run_source_descriptors_per_rule": (
        "MAX_PRIOR_RUN_SOURCE_DESCRIPTORS_PER_RULE",
        4_096,
        "usize",
    ),
    "max_prior_run_source_references": (
        "MAX_PRIOR_RUN_SOURCE_REFERENCES",
        65_536,
        "usize",
    ),
    "max_prior_run_source_manifest_bytes": (
        "MAX_PRIOR_RUN_SOURCE_MANIFEST_BYTES",
        16_777_216,
        "usize",
    ),
    "max_configuration_revision_bytes": (
        "MAX_CONFIGURATION_REVISION_BYTES",
        16_777_216,
        "usize",
    ),
    "max_completion_recovery_bytes": (
        "MAX_COMPLETION_RECOVERY_BYTES",
        512 * 1024,
        "usize",
    ),
    "max_provider_message_bytes": ("MAX_PROVIDER_MESSAGE_BYTES", 1_048_576, "usize"),
    "max_provider_deployment_routes": ("MAX_PROVIDER_DEPLOYMENT_ROUTES", 64, "usize"),
    "max_provider_proof_bytes": ("MAX_PROVIDER_PROOF_BYTES", 4 * 1024, "usize"),
    "max_provider_finish_authorization_bytes": (
        "MAX_PROVIDER_FINISH_AUTHORIZATION_BYTES",
        1_024,
        "usize",
    ),
    "max_fact_scan_publications": ("MAX_FACT_SCAN_PUBLICATIONS", 1_000_000, "u64"),
    "max_fact_scan_facts": ("MAX_FACT_SCAN_FACTS", 16_000_000, "u64"),
    "max_fact_scan_retained_source_bytes": (
        "MAX_FACT_SCAN_RETAINED_SOURCE_BYTES",
        512 * 1024 * 1024,
        "u64",
    ),
    "max_fact_scan_selected_results": ("MAX_FACT_SCAN_SELECTED_RESULTS", 16_384, "u64"),
    "max_fact_scan_response_bytes": (
        "MAX_FACT_SCAN_RESPONSE_BYTES",
        16 * 1024 * 1024,
        "u64",
    ),
    "max_fact_scan_distinct_producers": (
        "MAX_FACT_SCAN_DISTINCT_PRODUCERS",
        4_096,
        "u64",
    ),
    "max_fact_scan_producer_fold_batches": (
        "MAX_FACT_SCAN_PRODUCER_FOLD_BATCHES",
        16_000_000,
        "u64",
    ),
    "max_fact_scan_pages": ("MAX_FACT_SCAN_PAGES", 1_000_000, "u64"),
    "max_fact_selection_queries": ("MAX_FACT_SELECTION_QUERIES", 128, "usize"),
    "max_fact_selection_limit": ("MAX_FACT_SELECTION_LIMIT", 128, "u32"),
    "max_fact_emissions": ("MAX_FACT_EMISSIONS", 4_096, "usize"),
}

PORTABLE_ARTIFACT_HEX = "7b226b696e64223a226261746368222c226f7264696e616c223a302c227061796c6f6164223a7b226261746368223a7b22617070656e645f726571756573745f6964223a22706f727461626c652d676f6c64656e2d617070656e64222c2263616e6469646174655f646967657374223a22636f6e74656e743a7368613235362d76313a64623566613732383930366435373530323736373366356264343234313964616438313230393834653930646562353564383964346364383031393262313061222c2268656164223a7b22636f6d6d69745f646967657374223a227368613235362d6a63732d76313a37306634636338646239376130336537333338346339386636373134363062393536343433366462333032333165616433633462353466306432623362633862222c2272756e5f73657175656e6365223a317d2c226f626a65637473223a5b5d2c227072656465636573736f72223a6e756c6c2c227265636f726473223a5b7b227265636f7264223a7b226b696e64223a2272756e5f636c6f736564222c227061796c6f6164223a7b226f7574636f6d655f726566223a7b22636f6e74656e745f646967657374223a22636f6e74656e743a7368613235362d76313a31646363666136636561306338393637613761383134353533343031643362633735333436666464343835656362323235343366373061643132646536353233222c22736368656d615f6964223a22736368656d613a6d666d2e706f727461626c652d746573742e6f7574636f6d653a313a7368613235362d6a63732d76313a63333031323365373237303463323632376264636635373530376331663663353266386231626238633761353431373866653234633466633032316130353663227d7d7d2c227265636f72645f726566223a7b226f7264696e616c223a302c227265636f72645f68617368223a227368613235362d6a63732d76313a62393030623163643234396433373438343737613937353330346462346365656363616535323734346335373265646630393633313636336264616164646534222c2272756e5f6964223a2272756e3a7368613235362d6a63732d76313a63303961353037636566653965633633386636323036316263663461646264656331343232333262353032363639383265306130343765656165336136356432222c2272756e5f73657175656e6365223a317d7d5d2c2273746f72655f65706f6368223a2231222c2273746f72655f73636f70655f6964223a226d666d2e73746f72655f73636f70652e76313a3131313131313131313131313131313131313131313131313131313131313131222c2274656e616e745f666163745f636f6f7264696e617465223a7b226b696e64223a226e6f6e65227d7d2c2272756e5f6964223a2272756e3a7368613235362d6a63732d76313a63303961353037636566653965633633386636323036316263663461646264656331343232333262353032363639383265306130343765656165336136356432227d2c2270726576696f75735f6672616d655f646967657374223a6e756c6c7d0a7b226b696e64223a226261746368222c226f7264696e616c223a312c227061796c6f6164223a7b226261746368223a7b22617070656e645f726571756573745f6964223a22706f727461626c652d736f757263652d617070656e64222c2263616e6469646174655f646967657374223a22636f6e74656e743a7368613235362d76313a37396161663037306334303534623665386537396630306536393263313236663662326233623466356637353830303963376136316535643663633265616635222c2268656164223a7b22636f6d6d69745f646967657374223a227368613235362d6a63732d76313a66303431393033626331383836366537323666663037653534333330373364666265363034636162333265303339316538653834666331373339386632366231222c2272756e5f73657175656e6365223a317d2c226f626a65637473223a5b5d2c227072656465636573736f72223a6e756c6c2c227265636f726473223a5b7b227265636f7264223a7b226b696e64223a2272756e5f636c6f736564222c227061796c6f6164223a7b226f7574636f6d655f726566223a7b22636f6e74656e745f646967657374223a22636f6e74656e743a7368613235362d76313a31646363666136636561306338393637613761383134353533343031643362633735333436666464343835656362323235343366373061643132646536353233222c22736368656d615f6964223a22736368656d613a6d666d2e706f727461626c652d746573742e6f7574636f6d653a313a7368613235362d6a63732d76313a63333031323365373237303463323632376264636635373530376331663663353266386231626238633761353431373866653234633466633032316130353663227d7d7d2c227265636f72645f726566223a7b226f7264696e616c223a302c227265636f72645f68617368223a227368613235362d6a63732d76313a32373462373238613862386565346133376131336235626363333863643266383864663865326362613932633335363239353635393038666266363631326663222c2272756e5f6964223a2272756e3a7368613235362d6a63732d76313a35656562383137666432633765333835663930336531643664306564393930396533333735383862636234366562383335653262346138383562636361343633222c2272756e5f73657175656e6365223a317d7d5d2c2273746f72655f65706f6368223a2231222c2273746f72655f73636f70655f6964223a226d666d2e73746f72655f73636f70652e76313a3131313131313131313131313131313131313131313131313131313131313131222c2274656e616e745f666163745f636f6f7264696e617465223a7b2266726f6e74696572223a7b22666163745f6f72646572223a312c2273746f72655f65706f6368223a2231222c2273746f72655f73636f70655f6964223a226d666d2e73746f72655f73636f70652e76313a3131313131313131313131313131313131313131313131313131313131313131222c2274656e616e745f73636f70655f6964223a226d666d2e74656e616e745f73636f70652e76313a3232323232323232323232323232323232323232323232323232323232323232227d2c226b696e64223a22666163745f7075626c69636174696f6e227d7d2c2272756e5f6964223a2272756e3a7368613235362d6a63732d76313a35656562383137666432633765333835663930336531643664306564393930396533333735383862636234366562383335653262346138383562636361343633227d2c2270726576696f75735f6672616d655f646967657374223a22636f6e74656e743a7368613235362d76313a34336163643735656665363665393037323637663130343038336237376337366363366466313662303632663431626433326334333836336338623866386239227d0a7b226b696e64223a227365616c222c226f7264696e616c223a322c227061796c6f6164223a7b22617574686f72697a6174696f6e5f6465636973696f6e73223a5b7b226465636973696f6e5f726566223a22636f6e74656e743a7368613235362d76313a34363538643661626262616637373438633137326564356133653030336364623839393736343866383837323438333465343166373565353435323065313432222c226772616e74223a226578706f7274222c227072696e636970616c5f6964223a226d666d2e706f727461626c652d746573742f7072696e636970616c222c2272756e5f6964223a2272756e3a7368613235362d6a63732d76313a63303961353037636566653965633633386636323036316263663461646264656331343232333262353032363639383265306130343765656165336136356432227d2c7b226465636973696f6e5f726566223a22636f6e74656e743a7368613235362d76313a31633538653831653030643531343761363235346263626462313636313561613565656361323537363737616435643036393934656539643030643435666230222c226772616e74223a226578706f7274222c227072696e636970616c5f6964223a226d666d2e706f727461626c652d746573742f7072696e636970616c222c2272756e5f6964223a2272756e3a7368613235362d6a63732d76313a35656562383137666432633765333835663930336531643664306564393930396533333735383862636234366562383335653262346138383562636361343633227d5d2c22636c6f737572655f7265666572656e6365223a22636f6e74656e743a7368613235362d76313a35313763393833656237656137656636656436383036663437366264626565643032333765666531666265646136613532373934653939356162316364366466222c2266696e616c5f6672616d655f646967657374223a22636f6e74656e743a7368613235362d76313a65613764383039383465376666363161623039663334353338386634623238333263333631343435646265396566303763323133613035346131333235613434222c226669786174696f6e223a7b226a6f75726e616c5f68656164223a7b22636f6d6d69745f646967657374223a227368613235362d6a63732d76313a37306634636338646239376130336537333338346339386636373134363062393536343433366462333032333165616433633462353466306432623362633862222c2272756e5f73657175656e6365223a317d2c22706879736963616c5f746172676574223a7b2263757272656e745f696e6361726e6174696f6e5f726566223a22636f6e74656e743a7368613235362d76313a32313636366435386466316363666436373732333634383163663533343166373166336331373035366135636131636134346539386131646230306561656131222c2264617461626173655f6f6964223a372c2266656e63655f67656e65726174696f6e223a312c2272656c656173655f65706f6368223a312c227461726765745f6b6579223a22706f727461626c652d746573742d746172676574227d2c2273656d616e7469635f68656164223a7b2261646d697373696f6e5f726566223a7b226f7264696e616c223a302c227265636f72645f68617368223a227368613235362d6a63732d76313a62393030623163643234396433373438343737613937353330346462346365656363616535323734346335373265646630393633313636336264616164646534222c2272756e5f6964223a2272756e3a7368613235362d6a63732d76313a63303961353037636566653965633633386636323036316263663461646264656331343232333262353032363639383265306130343765656165336136356432222c2272756e5f73657175656e6365223a317d2c226b696e64223a2267656e65736973222c2273656d616e7469635f73746174655f646967657374223a227368613235362d6a63732d76313a64633261326230623162326163303534336431326562313666366461346335303866393163363664623630306631653233373933363238393336386638636431227d2c2273746f72655f65706f6368223a2231222c2273746f72655f73636f70655f6964223a226d666d2e73746f72655f73636f70652e76313a3131313131313131313131313131313131313131313131313131313131313131222c2274656e616e745f73636f70655f6964223a226d666d2e74656e616e745f73636f70652e76313a3232323232323232323232323232323232323232323232323232323232323232227d2c226672616d655f636861696e5f646967657374223a22636f6e74656e743a7368613235362d76313a61633939653161306339306434663338333834376231393438393539653633306537633836663966623061643234663166376431393536633539336530306666222c226b696e64223a2273656d616e746963222c22726f6f745f72756e5f6964223a2272756e3a7368613235362d6a63732d76313a63303961353037636566653965633633386636323036316263663461646264656331343232333262353032363639383265306130343765656165336136356432222c2272756e5f6669786174696f6e73223a5b7b22666163745f66726f6e7469657273223a5b5d2c22666163745f726f75746573223a5b7b22636f6e73756d65725f7265636f7264223a7b226f7264696e616c223a302c227265636f72645f68617368223a227368613235362d6a63732d76313a62393030623163643234396433373438343737613937353330346462346365656363616535323734346335373265646630393633313636336264616164646534222c2272756e5f6964223a2272756e3a7368613235362d6a63732d76313a63303961353037636566653965633633386636323036316263663461646264656331343232333262353032363639383265306130343765656165336136356432222c2272756e5f73657175656e6365223a317d2c2270726f64756365725f7472616e736974696f6e223a7b226f7264696e616c223a302c227265636f72645f68617368223a227368613235362d6a63732d76313a32373462373238613862386565346133376131336235626363333863643266383864663865326362613932633335363239353635393038666266363631326663222c2272756e5f6964223a2272756e3a7368613235362d6a63732d76313a35656562383137666432633765333835663930336531643664306564393930396533333735383862636234366562383335653262346138383562636361343633222c2272756e5f73657175656e6365223a317d2c227075626c69636174696f6e5f66726f6e74696572223a7b22666163745f6f72646572223a312c2273746f72655f65706f6368223a2231222c2273746f72655f73636f70655f6964223a226d666d2e73746f72655f73636f70652e76313a3131313131313131313131313131313131313131313131313131313131313131222c2274656e616e745f73636f70655f6964223a226d666d2e74656e616e745f73636f70652e76313a3232323232323232323232323232323232323232323232323232323232323232227d7d5d2c226669786174696f6e223a7b226a6f75726e616c5f68656164223a7b22636f6d6d69745f646967657374223a227368613235362d6a63732d76313a37306634636338646239376130336537333338346339386636373134363062393536343433366462333032333165616433633462353466306432623362633862222c2272756e5f73657175656e6365223a317d2c22706879736963616c5f746172676574223a7b2263757272656e745f696e6361726e6174696f6e5f726566223a22636f6e74656e743a7368613235362d76313a32313636366435386466316363666436373732333634383163663533343166373166336331373035366135636131636134346539386131646230306561656131222c2264617461626173655f6f6964223a372c2266656e63655f67656e65726174696f6e223a312c2272656c656173655f65706f6368223a312c227461726765745f6b6579223a22706f727461626c652d746573742d746172676574227d2c2273656d616e7469635f68656164223a7b2261646d697373696f6e5f726566223a7b226f7264696e616c223a302c227265636f72645f68617368223a227368613235362d6a63732d76313a62393030623163643234396433373438343737613937353330346462346365656363616535323734346335373265646630393633313636336264616164646534222c2272756e5f6964223a2272756e3a7368613235362d6a63732d76313a63303961353037636566653965633633386636323036316263663461646264656331343232333262353032363639383265306130343765656165336136356432222c2272756e5f73657175656e6365223a317d2c226b696e64223a2267656e65736973222c2273656d616e7469635f73746174655f646967657374223a227368613235362d6a63732d76313a64633261326230623162326163303534336431326562313666366461346335303866393163363664623630306631653233373933363238393336386638636431227d2c2273746f72655f65706f6368223a2231222c2273746f72655f73636f70655f6964223a226d666d2e73746f72655f73636f70652e76313a3131313131313131313131313131313131313131313131313131313131313131222c2274656e616e745f73636f70655f6964223a226d666d2e74656e616e745f73636f70652e76313a3232323232323232323232323232323232323232323232323232323232323232227d2c2272756e5f6964223a2272756e3a7368613235362d6a63732d76313a63303961353037636566653965633633386636323036316263663461646264656331343232333262353032363639383265306130343765656165336136356432222c2274656e616e745f73636f70655f6964223a226d666d2e74656e616e745f73636f70652e76313a3232323232323232323232323232323232323232323232323232323232323232227d2c7b22666163745f66726f6e7469657273223a5b7b22666163745f6f72646572223a312c2273746f72655f65706f6368223a2231222c2273746f72655f73636f70655f6964223a226d666d2e73746f72655f73636f70652e76313a3131313131313131313131313131313131313131313131313131313131313131222c2274656e616e745f73636f70655f6964223a226d666d2e74656e616e745f73636f70652e76313a3232323232323232323232323232323232323232323232323232323232323232227d5d2c22666163745f726f75746573223a5b5d2c226669786174696f6e223a7b226a6f75726e616c5f68656164223a7b22636f6d6d69745f646967657374223a227368613235362d6a63732d76313a66303431393033626331383836366537323666663037653534333330373364666265363034636162333265303339316538653834666331373339386632366231222c2272756e5f73657175656e6365223a317d2c22706879736963616c5f746172676574223a7b2263757272656e745f696e6361726e6174696f6e5f726566223a22636f6e74656e743a7368613235362d76313a32313636366435386466316363666436373732333634383163663533343166373166336331373035366135636131636134346539386131646230306561656131222c2264617461626173655f6f6964223a372c2266656e63655f67656e65726174696f6e223a312c2272656c656173655f65706f6368223a312c227461726765745f6b6579223a22706f727461626c652d746573742d746172676574227d2c2273656d616e7469635f68656164223a7b2261646d697373696f6e5f726566223a7b226f7264696e616c223a302c227265636f72645f68617368223a227368613235362d6a63732d76313a32373462373238613862386565346133376131336235626363333863643266383864663865326362613932633335363239353635393038666266363631326663222c2272756e5f6964223a2272756e3a7368613235362d6a63732d76313a35656562383137666432633765333835663930336531643664306564393930396533333735383862636234366562383335653262346138383562636361343633222c2272756e5f73657175656e6365223a317d2c226b696e64223a2267656e65736973222c2273656d616e7469635f73746174655f646967657374223a227368613235362d6a63732d76313a39663532633865653561633366653635336630356534323236383561663935333937343162363737353833313839396439306664383839646165666233353765227d2c2273746f72655f65706f6368223a2231222c2273746f72655f73636f70655f6964223a226d666d2e73746f72655f73636f70652e76313a3131313131313131313131313131313131313131313131313131313131313131222c2274656e616e745f73636f70655f6964223a226d666d2e74656e616e745f73636f70652e76313a3232323232323232323232323232323232323232323232323232323232323232227d2c2272756e5f6964223a2272756e3a7368613235362d6a63732d76313a35656562383137666432633765333835663930336531643664306564393930396533333735383862636234366562383335653262346138383562636361343633222c2274656e616e745f73636f70655f6964223a226d666d2e74656e616e745f73636f70652e76313a3232323232323232323232323232323232323232323232323232323232323232227d5d2c22736f757263655f72756e5f696473223a5b2272756e3a7368613235362d6a63732d76313a35656562383137666432633765333835663930336531643664306564393930396533333735383862636234366562383335653262346138383562636361343633225d2c2273746f72655f73636f70655f6964223a226d666d2e73746f72655f73636f70652e76313a3131313131313131313131313131313131313131313131313131313131313131222c2274656e616e745f73636f70655f6964223a226d666d2e74656e616e745f73636f70652e76313a3232323232323232323232323232323232323232323232323232323232323232222c22746f74616c5f6279746573223a383034342c22746f74616c5f6672616d6573223a332c2276657273696f6e223a226d666d2e737472756374757265642d706f727461626c652d72756e2d6578706f72742d73747265616d2e7632227d2c2270726576696f75735f6672616d655f646967657374223a22636f6e74656e743a7368613235362d76313a65613764383039383465376666363161623039663334353338386634623238333263333631343435646265396566303763323133613035346131333235613434227d0a"


def generated_limits_source() -> str:
    lines = [
        "//! Generated recoverability budgets; edit `contracts/recoverability/generate.py`.",
        "",
    ]
    for annex_name, (rust_name, value, rust_type) in LIMITS.items():
        lines.extend(
            [
                f"/// Generated `{annex_name}` budget.",
                f"pub const {rust_name}: {rust_type} = {value};",
                "",
            ]
        )
    return "\n".join(lines)


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
                                field("maximum_distinct_producers", unsigned(4_096, 1)),
                                field(
                                    "maximum_producer_fold_batches",
                                    unsigned(16_000_000, 1),
                                ),
                                field("maximum_pages", unsigned(1_000_000, 1)),
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
        "mfm.structured-configuration-revision.v1": (
            ["P-AP-01", "P-RH-01"],
            object_shape(
                [
                    field("append_request_id", string("valid_unicode_scalar_string", 1, 512)),
                    field(
                        "canonical_value",
                        string(
                            "valid_unicode_scalar_string",
                            1,
                            LIMITS["max_configuration_revision_bytes"][1],
                        ),
                    ),
                    field("key", canonical_value),
                    field("predecessor_ref", nullable(content_ref)),
                    field("revision_ref", content_ref),
                    field("sequence", unsigned(18_446_744_073_709_551_615, 1)),
                    field("value_contract_ref", content_ref),
                    field("value_ref", content_ref),
                ],
                "revision_ref binds the exact stream key, predecessor, append identity, and value",
                "canonical_value is the exact retained float-free configured value",
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
                        string(
                            "valid_unicode_scalar_string",
                            1,
                            LIMITS["max_stored_frame_bytes"][1],
                        ),
                    ),
                    field("content_ref", content_ref),
                    field("object_type", reference("mfm.primitive-stable_id.v1")),
                ],
                "content_ref hashes the exact canonical_json UTF-8 bytes",
            ),
        ),
        "mfm.portable-physical-target.v1": (
            ["P-AP-01", "P-RH-01"],
            object_shape(
                [
                    field("current_incarnation_ref", reference("mfm.primitive-content_digest.v1")),
                    field("database_oid", unsigned(4_294_967_295)),
                    field("fence_generation", unsigned(18_446_744_073_709_551_615)),
                    field("release_epoch", unsigned(18_446_744_073_709_551_615)),
                    field("target_key", string("valid_unicode_scalar_string", 1, 512)),
                ],
                "the target identity binds the exact physical database, fence, release, and current incarnation",
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
                    field("tenant_scope_id", reference("mfm.primitive-tenant_scope_id.v1")),
                    field("physical_target", reference("mfm.portable-physical-target.v1")),
                ],
                "semantic and physical fixation bind one exact export prefix",
            ),
        ),
        "mfm.portable-authorization-decision.v1": (
            ["P-AP-01"],
            object_shape(
                [
                    field("decision_ref", reference("mfm.primitive-content_digest.v1")),
                    field("grant", literal("export")),
                    field("principal_id", reference("mfm.primitive-stable_id.v1")),
                    field("run_id", reference("mfm.primitive-run_id.v1")),
                ],
                "one exact export grant decision is retained for each fixed run prefix",
            ),
        ),
        "mfm.portable-run-export-frame.v1": (
            ["P-AP-01", "P-RH-01"],
            object_shape(
                [
                    field("kind", enum("batch", "seal")),
                    field("ordinal", unsigned(1_048_576)),
                    field("payload", canonical_value),
                    field(
                        "previous_frame_digest",
                        nullable(reference("mfm.primitive-content_digest.v1")),
                    ),
                ],
                "each frame is one exact float-free canonical JSON line",
                "ordinal is dense and previous_frame_digest links the frame chain",
            ),
        ),
        "mfm.portable-run-fixation.v1": (
            ["P-AP-01", "P-RH-01"],
            object_shape(
                [
                    field("fact_frontiers", array(canonical_value)),
                    field("fact_routes", array(canonical_value)),
                    field("fixation", reference("mfm.portable-fixation.v1")),
                    field("run_id", reference("mfm.primitive-run_id.v1")),
                    field("tenant_scope_id", reference("mfm.primitive-tenant_scope_id.v1")),
                ],
                "run fixation binds every selected route and dense frontier to one exact prefix",
            ),
        ),
        "mfm.portable-run-export-stream.v1": (
            ["P-AP-01", "P-RH-01"],
            object_shape(
                [
                    field(
                        "authorization_decisions",
                        array(reference("mfm.portable-authorization-decision.v1"), 1, 4_097),
                    ),
                    field("closure_reference", reference("mfm.primitive-content_digest.v1")),
                    field("final_frame_digest", reference("mfm.primitive-content_digest.v1")),
                    field("frame_chain_digest", reference("mfm.primitive-content_digest.v1")),
                    field("fixation", reference("mfm.portable-fixation.v1")),
                    field("kind", enum("semantic", "audit")),
                    field("root_run_id", reference("mfm.primitive-run_id.v1")),
                    field(
                        "run_fixations",
                        array(reference("mfm.portable-run-fixation.v1"), 1, 4_097),
                    ),
                    field("source_run_ids", array(reference("mfm.primitive-run_id.v1"))),
                    field("store_scope_id", reference("mfm.primitive-store_scope_id.v1")),
                    field("tenant_scope_id", reference("mfm.primitive-tenant_scope_id.v1")),
                    field("total_bytes", unsigned(16_777_216)),
                    field("total_frames", unsigned(1_048_577)),
                    field("version", literal("mfm.structured-portable-run-export-stream.v2")),
                ],
                "the wire value is a newline-delimited stream of registered frame values",
                "the terminal seal binds the complete frame chain and authorized closure",
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
            "max_canonical_json_bytes": str(LIMITS["max_canonical_json_bytes"][1]),
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
            **{name: str(value) for name, (_, value, _) in LIMITS.items()},
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
                "CommittedAccessAuthorization",
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
            "maximum_distinct_producers": 4_096,
            "maximum_producer_fold_batches": 16_000_000,
            "maximum_pages": 1_000_000,
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
            "mfm.portable-run-export-frame.v1",
            {
                "kind": "batch",
                "ordinal": 0,
                "payload": {},
                "previous_frame_digest": None,
            },
        ),
        schema_vector(
            annex,
            "mfm.portable-authorization-decision.v1",
            {
                "decision_ref": f"content:sha256-v1:{'3' * 64}",
                "grant": "export",
                "principal_id": "mfm.recoverability/principal",
                "run_id": run_id,
            },
        ),
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
            "expected_error": "unknown_field",
            "id": "codec/unknown-field/portable-frame-legacy",
            "input_hex": canonical(
                {
                    "kind": "batch",
                    "legacy": True,
                    "ordinal": 0,
                    "payload": {},
                    "previous_frame_digest": None,
                }
            ).hex(),
            "kind": "schema_rejection",
            "schema_contract": "mfm.portable-run-export-frame.v1",
        },
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
    portable_artifact = bytes.fromhex(PORTABLE_ARTIFACT_HEX)
    portable_frames = portable_artifact.splitlines()
    if len(portable_frames) != 3:
        raise ValueError("portable artifact fixture must contain root, source, and seal frames")
    omitted_source = b"\n".join([portable_frames[0], portable_frames[2]]) + b"\n"
    extra_source = b"\n".join(
        [portable_frames[0], portable_frames[1], portable_frames[1], portable_frames[2]]
    ) + b"\n"
    reordered_source = b"\n".join(
        [portable_frames[1], portable_frames[0], portable_frames[2]]
    ) + b"\n"
    substituted_source = bytearray(portable_artifact)
    source_marker = b"portable-source-append"
    marker_offset = substituted_source.find(source_marker)
    if marker_offset < 0:
        raise ValueError("portable artifact source marker is absent")
    substituted_source[marker_offset] = ord("x")
    portable_artifact_vectors = [
        {
            "bytes_hex": portable_artifact.hex(),
            "id": "portable/recursive-golden/accept",
            "kind": "strict_decode_acceptance",
        },
        {
            "bytes_hex": omitted_source.hex(),
            "expected_error": "invalid",
            "id": "portable/recursive-golden/omitted-source",
            "kind": "strict_decode_rejection",
        },
        {
            "bytes_hex": extra_source.hex(),
            "expected_error": "invalid",
            "id": "portable/recursive-golden/extra-source",
            "kind": "strict_decode_rejection",
        },
        {
            "bytes_hex": reordered_source.hex(),
            "expected_error": "invalid",
            "id": "portable/recursive-golden/reordered-source",
            "kind": "strict_decode_rejection",
        },
        {
            "bytes_hex": bytes(substituted_source).hex(),
            "expected_error": "invalid",
            "id": "portable/recursive-golden/substituted-source",
            "kind": "strict_decode_rejection",
        },
    ]
    return {
        "artifacts": {
            "annex_bytes": len(annex_bytes),
            "annex_sha256": hashlib.sha256(annex_bytes).hexdigest(),
        },
        "contract": "mfm.recoverability-corpus.v1",
        "negative_vectors": negative,
        "portable_artifact_vectors": portable_artifact_vectors,
        "positive_vectors": positives,
        "relational_vectors": relational,
    }


def main() -> None:
    annex = build_annex()
    annex_bytes = canonical(annex)
    corpus_bytes = canonical(build_corpus(annex, annex_bytes))
    (ROOT / "annex.json").write_bytes(annex_bytes)
    (ROOT / "corpus.json").write_bytes(corpus_bytes)
    (Path(__file__).resolve().parents[2] / "crates/kernel/canonical/src/recoverability_limits.rs").write_text(
        generated_limits_source(), encoding="utf-8"
    )


if __name__ == "__main__":
    main()
