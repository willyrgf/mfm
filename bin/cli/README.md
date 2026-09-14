# MFM CLI

`mfm_cli` is the command-line rendering of the typed Application client surface. It owns bounded
argv/file/stdin parsing, text/JSON rendering, and CLI-only schema provisioning. It does not parse
deployment TOML, resolve locator environments, construct providers, plan domain Programs, or
interpret run history.

The CLI adds no authentication or authorization layer. Process execution, environment access, and
database credentials belong to the enclosing deployment.

## Commands

```text
mfm_cli [--output text|json] entry-point list
mfm_cli [--output text|json] inspect list

mfm_cli [--deployment <PATH>] [--output text|json] postgres init \
    --admin-locator-env <NAME>
mfm_cli [--deployment <PATH>] [--output text|json] binding list

mfm_cli [--deployment <PATH>] [--output text|json] config import <NAME> --from <PATH|->
mfm_cli [--deployment <PATH>] [--output text|json] config publish-enrichment <NAME> --run-id <RUN_ID>
mfm_cli [--deployment <PATH>] [--output text|json] config list
mfm_cli [--deployment <PATH>] [--output text|json] config delete <NAME> --digest <DIGEST>

mfm_cli [--deployment <PATH>] [--output text|json] run start --config <NAME> \
    --config-digest <DIGEST> [--run-id <RUN_ID>]
mfm_cli [--deployment <PATH>] [--output text|json] run progress --run-id <RUN_ID>
mfm_cli [--deployment <PATH>] [--output text|json] run show --run-id <RUN_ID>
mfm_cli [--deployment <PATH>] [--output text|json] run list [--after <RUN_ID>] [--limit <N>]
```

`entry-point list` and `inspect list` are static and reject `--deployment`. Every other command loads
the override or the conventional XDG/HOME `deployment.toml`. `postgres init` additionally resolves
the checked admin locator name and retains no administrative handle after provisioning. The complete
deployment grammar, bounds, and example are documented by
[`mfm-app`](../../crates/app/README.md).

`inspect list` is the crude developer inventory of every entry point, reusable Operation, and
Pure/Read State admitted by the compiled product composition. It does not expand an Operation or
load a configuration. IDs and kinds are stable machine fields; descriptions are non-semantic
human-facing prose. Component inspection is intentionally CLI-only.

`config import` reads at most 256 KiB plus one byte from a file or stdin. It creates one immutable
name/digest revision or leaves identical retained bytes unchanged. Different digests under one name
remain independent. `config delete` idempotently removes only the exact name/digest pair. The stored
document is a complete tagged execution config; `run start` accepts no entry-point or inline
document and always requires one exact retained digest.

Without `--run-id`, the CLI uses the shared Application client primitive that obtains exactly 32
bytes from the OS cryptographic random source and applies the frozen derivation. Entropy failure is
`run_id_generation_failed`; there is no time, PID, counter, environment, provider, or existence
fallback. Explicit RunIds bypass generation and support deterministic retries.

## Output

`--output text` is the default human presentation. Listed config revisions include `config_name`,
`config_digest`, and `entry_point`. Start prints those fields before the run fields. Successful run
text includes `contract_ref`, `value_ref`, and exact canonical `value`. Failed runs include
`value_ref` and the canonical `report`; runnable/pending views expose position, reason or EffectId.
Awaiting-recovery views include the original failure and complete executed input and intent/command
facts. Awaiting-interpretation views include the retained command and accepted settlement evidence.
Text renders all state fields from the prepared shared model, preserving raw canonical JSON.

`--output json` is the stable automation surface shared with REST. It preserves the documented
Application models: generic item lists, config revision summaries, start results, mechanical run
pages, and the full tagged RunView. Terminal values occupy raw JSON positions rather than quoted
strings. Successful bodyless operations emit `{}` in JSON mode
and nothing in text mode.

Component inspection JSON is an `ItemList` whose items contain `kind`, `id`, and `description`.
Text renders the same three fields for each component.

Ordinary JSON errors are exactly `{"code":"...","message":"..."}`. The Application-owned client
error serializer adds `recovery` and the complete tagged `invocation` for an ambiguous append,
or `invocation` alone for an execution-stopped call or stopped pending-Effect recovery.
Ambiguous appends retain `run_append_indeterminate`; known noninsertion uses
`run_append_not_inserted` even when a later probe has a different failure. A null
last observation means no qualified head is known. Text prints the same recovery identity and
invocation details after `error: <message>`. These errors do not claim a durable terminal state. MFM does not
append secrets, configuration bodies or full requests/connections to diagnostics. Canonical reports
retain provider method, stage, status/code, ordered source messages and selected fields under the
upstream diagnostic trust contract. No independent diagnostic quota or omission ledger applies.


`source_cycle: true` means exactly “traversal stopped on a repeated interface pointer.” The local
source walker compares complete `dyn Error` pointers with `std::ptr::eq`; it does not establish
concrete-object identity. An inline child may share its parent's data address, and one concrete
error can have different interface representations. The latter may produce repeated cause entries
before termination; no exact cyclic-object visit count is promised across compiler configurations.
There is no identity registry, message comparison or diagnostic budget.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | A non-run command succeeded, or a run view is `succeeded`. |
| 1 | A run view is `runnable`, `effect_pending`, `awaiting_recovery`, `awaiting_interpretation` or durably `failed`. |
| 2 | A call failed: usage, input, composition, entropy, request, stopped invocation or output failure; any last observed view is historical. |

The old `init`, `snapshot`, and `show --config` grammar and combined configuration file do not
exist. Keystore administration and transaction submission remain outside this surface.

The managed `client-e2e` task builds both binaries explicitly, admits an exact historical run by interrupting
an in-flight live Read, deletes its config revision, resumes it through the Unix-socket
REST listener against Reth, validates the complete snapshot, and reloads the same durable RunView
through this JSON surface. It then reimports the same revision, starts a fresh CLI-generated run, and
requires an identical semantic result from the independent execution. Both renderers also match the
frozen start/progress recovery envelopes in `docs/contracts/client-surface/`.

Candidate discovery is selected by a `mfm.portfolio/enrich@1` configuration and uses ordinary
`run start/show/progress`. `config publish-enrichment` explicitly publishes a successful result as
a snapshot revision. Repetition returns the same revision without discovery. Start of an admitted
RunId with the same selection works after config deletion; a different revision conflicts.

Size-limit invocation failures use `size_limit_exceeded` and include
`invocation.size_limit` with `resource` and `limit` (bytes, or frames for
`frame_count`). Measured violations carry `actual`; a serializer stopped at its ceiling carries
`observed_at_least` without claiming a measured final size. The last observation remains historical; oversized inline reports do not
append a terminal conclusion or discard pending Effect authority. Capacity arithmetic overflow
uses `capacity_arithmetic_overflow` without fabricated measurements. CLI uses exit 2; REST uses
422 for these stopped invocations. Response payloads retain the shared full inline report; request
body limits do not impose a response-size limit.

Pending Effect views include `latest_failure`: null before the first audited failure, otherwise
`mode: "effect"`, qualified `error`, complete `input`, retained `command`, `effect_id`, and `decision` (`"retry"` or
`{"stop":{"reason":"requested"}}`, with other reviewed stop codes). Every acknowledged
pending operational outcome advances the durable head. Retry preserves the command/EffectId and
spends recovery allowance; Stop ends the invocation while explicit progress may resume it.
There is no separate pending-failure quota; actual frame and run limits govern recording.
Cancellation can interrupt a physical
attempt before its result is recorded; the audit covers acknowledged qualified failures.


Run emitters retain their concrete result through output preparation, write and flush. They return
an exit code directly. A normal encoding failure or stdout write/flush failure gets one final stderr
presentation and exit 2. Failure of a normal or final stderr presentation ends without another write.
The shared [App failure presentation](../../crates/app/README.md#report-preparation-failures) preserves
the original request category and recovery identity, historical `last_observed` head, explicit
Runtime `acknowledged` head and available candidate/size facts.

JSON output embeds the exact already encoded value as `original_report`; this is null if normal
encoding never completed. Text output borrows typed fields directly, and a stdout failure includes
its existing buffer as `original_text`. Structured field values are JSON on individual text lines;
there is no JSON-to-text parse round trip. The output diagnostic retains stream, write/flush stage,
top IO message, OS kind/code and ordered exposed custom source messages. Repeated full interface pointers
end traversal with `source_cycle: true`, under the information limit above; no independent diagnostic quota applies. MFM does not
append rejected buffers or credentials to source details. Successful local write/flush does not
prove reader consumption, and no output failure creates a Journal record.

Malformed stored framework Objects are reported as `internal`, with restore/decode parser category,
available line/column and rejection reason. This includes nested Object-size rejection: no structured
size violation is inferred from parser text. CLI exits 2 and REST returns 500. Direct typed size
failures retain their structured fields and REST 422 treatment; postdecode slot mismatches retain
identity fields. JSON diagnostics preserve dependency-supplied text without adding secret inputs.

Runtime persists one current operation and derives its public phase. Terminal reports borrow the
retained original and mapped domain root; pending recovery reports borrow the unchanged input,
command and original from their observed Effect. Their JSON field names and status rules remain
unchanged. There is no duplicate owned incident or terminal cause tree in the public library API.

Run Store failures inside the shared invocation payload now carry mandatory diagnostic data, for
example `{"unavailable":{"operation":"run.load","stage":"begin","sources":[...]}}`.
The existing public code/status and acknowledgement meaning remain unchanged. Clients forward
these producer facts through the shared Application presentation; they do not recapture sources
or claim the failed Store durably recorded the invocation.
