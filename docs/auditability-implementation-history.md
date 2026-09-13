# Auditability implementation history

Background record, reviewed 2026-09-13. This note preserves the diagnosis and measurements behind
[RFC Part 1](../RFC_AUDITABILITY_ERROR_CHAIN.md) and
[RFC Part 2](../RFC_CAUSAL_ERROR_PRESERVATION.md). It is optional background, not an implementation
plan, acceptance record, or source of additional scope. The current RFCs own the target design,
allowed work, acceptance criteria and outstanding proofs. Read their current versions to implement.

## Repository chronology

| Reference | Historical role |
| --- | --- |
| 7f71beef | Original code comparison baseline. |
| 377f649f | RFC revision used by the third implementation attempt. |
| 6ea98576 | Initial deletion steps completed. |
| aae81795 | Retained core implementation. |
| a3e8fd2c plus reviewed worktree | Later candidate measured in /tmp/mfm-audit-core-first. |
| 1a6bcbc5 | Split RFCs carried onto the retained core after branch cleanup. |
| 5de114d0 | Typed owner-error and boundary-diagnostic agreements incorporated into both RFCs; production code unchanged from the retained core. |

Earlier branch/worktree changes remain archived, including /tmp/mfm-audit-core-first. The branch
cleanup retained the core instead of restarting implementation. Historical patches may supply a
specific reviewed change or test, but archived work is not the current implementation plan and
must not be restored wholesale. This document reorganization discards no source or worktree.

The former combined RFC's full-handoff verdict was withdrawn. Splitting the deliverable did not
establish the outstanding core proofs. At this review, no final integrated implementation/CI or
achieved net simplification was established. Old documentation and disposable probes are evidence
of what was tried, not accepted current contracts.

## Recorded diagnosis

The following diagnosis was moved from Part 1. Its correction column records the review's
conclusions; implementation details are governed by the current RFC, not by this historical table.
References to numbered RFC sections refer to the reviewed RFC revision.

The third attempt did complete the initial deletion steps. The expansion occurred in both the core
replacement and the later error-owner migrations. Calling it an unfinished step 2 is inaccurate;
calling every addition unnecessary is also unsupported. The following measurements were reproduced
from `/tmp/mfm-audit-core-first` at `a3e8fd2c` plus its reviewed worktree on 2026-09-13.

### Measured costs and their limits

| Checkpoint | Net production Rust code against 7f71beef |
| --- | ---: |
| Steps 1-2, 6ea98576 | -630 |
| Core, aae81795 | +1,222 |
| Reviewed later candidate, including untracked production files | +5,998 |
| Growth after the core | +4,776 |

The reproduced counter counts nonblank, non-comment production `src` lines, excluding separate
and trailing inline tests. It is a convention, not a complexity metric or a Rust syntax analysis.
The exact tracked diff against `377f649f` is **223 files, 25,713 additions, 10,442 deletions**;
untracked files are additional to that Git total. Tests and documentation are real migration cost,
even though they are excluded from production-code counts.

Post-core production growth concentrates in PostgreSQL (+1,590), Application (+830), REST (+426),
Runtime (+400), Store (+331), live EVM (+282), and the remaining owners (+917). The core itself
added 1,852 production lines after the 630-line initial reduction. Thus the later owner backlog
is not the sole explanation, and making that backlog finite would not by itself simplify the core.

The second attempt's 189-file/+20,033 tracked-line net growth and the third attempt's measurements
are separate historical observations. Neither is a deletion allowance for the replacement.

### Obligations and mechanisms that caused growth

| Requirement in the previous revision | Actual expansion it enabled or required | Required correction |
| --- | --- | --- |
| RunCommit stores RunState.phase and OperationFacts | The same failure, command, settlement or output is serialized twice; the validator reconstructs a phase and checks agreement. The 648-line fold was deleted, but state.rs added 927 physical lines and state/decode.rs another 294. Those files also contain necessary logic; their entire size is not deletion credit. | Store one current record whose operation determines continuation. Delete the independent phase and its agreement checks. Keep actual authorization and current-record admission. |
| Ordinary Serde plus exact native constructor causes at nested Object admission | ObjectSeed and a macro/seed for every enclosing state struct, enum and collection duplicate the payload grammar, add drain/error-precedence logic, and require parallel wire tests. The previous probe tested shape-only objects and did not exercise this conflict. | Prove parse-then-admit with actual Objects and adapter signatures. Do not claim ordinary Serde preserves arbitrary nested constructor causes. Section 6 records the API tradeoff and blocking proof. |
| NativeCause retains a fallible reporting operation | project() calls child project(), serializes an owner Wire, parses RawValue, and can return another NativeCause. Store, Runtime, config and PostgreSQL acquire companion projectors; App retains failures of secondary projection. | Delete NativeCause, its owner protocol and projectors. Adapt selected fields once at the concrete boundary; receivers forward data without another capture or reporting-error tree. |
| Preserve arbitrary native E even when its only generic extraction method fails | The failure encoder creates erased ownership and Arc custody; later consumers need downcasting or another projector. Successful admission does not remove the exceptional custody requirement. | Explicitly report unavailable original detail on failed initial encoding. Keep the existing typed producer API; add no universal fallback payload, Error bound or opaque owner. |
| Consumers rediscover owner facts during reporting | Runtime walks and downcasts Value/Journal/Store/Canonical errors for size facts; CLI serializes its own report and parses it back to render text, creating Fields/MissingField failures. | Supply primary size facts at the owner boundary and render both formats from the same data. Delete source inspection and JSON-to-text reconstruction. |
| Preserve originals while requiring every reachable native diagnostic to be certified secret-free | Parser/client objects and native downcasts triggered owner audits, sanitizers and reporting work beyond the selected execution failures. | Trust dependency diagnostic content under section 4. Stop deliberately attaching MFM secret inputs; remove blanket upstream getter/source certification and per-caller sanitizers from the delivery obligation. |
| Close every remaining first-loss owner and every consumer | A real caller justified another constructor migration, then another serializer, transport path and test family. C15 had no finite completion set. | Close Part 1 with its finite core cases. Assign selected execution producer enrichment to Part 2, refined against the accepted core; unrelated platform remediation is outside both RFCs. Existing cause forwarding closes a changed API; extending an upstream contract does not silently expand Part 1. |
| Report LOC and explain increases before owner fanout | The engineer supplied measurements and a local rationale, explicitly without establishing overall minimality, then continued. An explanation functioned as acceptance. | Require independent acceptance of the actual corrected core on its own guarantees and cost. Part 2 has a separate design/cost gate after that acceptance. A failed core review does not authorize owner work. |
| Preserve exact errors from capture and reporting themselves | Auxiliary field validation and projection failures became new error families with their own projections and custody tests. | Use bounded, fixed capture-status/omission data for diagnostic construction. They describe missing evidence; they are not new operational causes or another extensible reporting subsystem. |

The RFC author is responsible for these obligations and for the overly strong handoff verdict.
The engineer also continued after acknowledging unresolved aggregate size, but the design supplied
multiple routes by which that continuation appeared compliant. More instructions to prefer fewer
lines, another clean checkout, or another local source-preservation review would not fix this.

### What the previous verification established

The ten disposable probe tests established enum wire shape and some ownership mechanics. They did
not exercise checked Object admission inside the real nested payload, shared error serialization
across owners, rejected-data custody, or an integrated deletion. Passing them was insufficient
basis for the previous claim that no important design decisions remained.

The third attempt contains useful consuming tests, frame/load measurements and real deletions.
They are evidence to preserve and reassess against the corrected contract, not proof that the
whole attempt is minimal or complete. Final integrated CI was not established by this review.

## Historical scope mapping

The combined owner inventory was reduced to selected execution paths. This paragraph records the
old-to-split scope mapping; it creates no implementation rows in either current RFC:

The old O5 and O7 producer migrations are removed. O8 is limited to changed run-Store consumers
within O4; its configuration/index work is excluded. O9/O10 are only forwarding/rendering of the
selected run results and actual terminal encoding/write/flush failures; their broad producer
migrations are removed. Reuse Part 1's shared report route and statuses. O12 extraction is a named
dependency of a retained case, not a separate open-ended owner row. O11 remains Part 1 E5.

The AuthorityFailure and SigningFailure names in an earlier sketch were undefined payload
placeholders, not approved new error types. The current Part 2 refinement selects concrete owner
errors or the minimal required durable fields. NoParams already existed behind framework_unit!;
moving its declaration into ordinary source was not introduction of a new concept.

## Verification limits

The historical counters exclude comments, blanks and identified tests under the convention stated
above. Tracked Git churn and production-code deltas measure different things. Neither identifies
all unnecessary code or proves the cost of its replacement. Removing code found only in an
abandoned branch earns no deletion credit against a baseline where it never existed.

The current RFCs retain the resulting controls: finite producer/consumer scope, explicit type
ownership and deletions, proofs through actual execution boundaries, cumulative cost accounting,
and independent acceptance before further scope. Those controls apply without rereading this note.

## Material uncertainties

Historical measurements describe the named commits and reviewed worktree, not a completed target
implementation. The assumption that the replacement produces a net simpler design remains
unverified until the current RFC's implementation and acceptance proofs complete. Validate that
assumption against the pinned comparison baselines and actual final candidate; do not extrapolate
an achieved reduction from these historical figures.
