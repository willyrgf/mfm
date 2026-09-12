# Current-state representation measurements

Measured on 2026-09-12 during the core cutover from `7f71beef`, using Program v8, frame v6, and
FailureReport v4. These measurements exercise the production Runtime and memory Store through
`start`, `read`, and `resume`. They do not use an independent state model or prospective estimator.
The provider and authority fixtures return the baseline operational payloads; their upstream
capture gaps remain assigned to RFC step 4.

## Encoded frames

The [per-frame CSV](auditability-core-frames.csv) records actual encoded bytes. All sizes below
are bytes. `phase_object_bytes`, `checkpoint_bytes`, and `fact_object_bytes` sum the canonical
payload of every inline Object occurrence in their respective current-record fields. Admission
facts include the Program and initial context. `object_occurrences` counts all inline occurrences;
`repeated_occurrences` counts occurrences after the first matching complete value reference.
Metadata is the complete encoded frame length minus all those canonical payload bytes. References,
tags, positions, usage, and the Journal envelope remain metadata.

| Fixture | Frames | Admission | Largest frame | Largest metadata | History total |
| --- | ---: | ---: | ---: | ---: | ---: |
| Portfolio success | 10 | 48,495 | 48,495 | 2,307 | 96,083 |
| Portfolio domain failure | 5 | 48,495 | 48,495 | 3,589 | 70,349 |
| Anchored Read success | 2 | 12,877 | 12,877 | 2,036 | 23,961 |
| Anchored Read domain failure | 3 | 12,877 | 18,028 | 3,405 | 45,689 |
| Transaction success | 11 | 27,533 | 27,533 | 2,682 | 104,728 |
| Transaction operational stop and explicit resume | 13 | 27,533 | 27,533 | 2,714 | 115,325 |
| Two identical checkpoint contexts | 4 | 14,070 | 14,070 | 2,127 | 30,249 |
| Two distinct checkpoint contexts | 4 | 14,070 | 14,070 | 2,127 | 30,249 |
| Anchored Read with one retry | 4 | 12,879 | 12,879 | 2,366 | 39,450 |
| Anchored Read with five retries | 12 | 12,879 | 12,879 | 2,367 | 101,401 |

The Portfolio fixture uses `plan_snapshot` with one native-balance source, the shipping
`PortfolioContinuation` and State types, and controlled adapters. Its failure case returns rejected
chain-identity evidence. The anchored fixture uses the checked completed-call workflow from
`anchored_call_contract.rs` with the public `alpha` endpoint identity. Its failure case returns
rejected anchored evidence. The transaction fixture uses `EvmTransaction`, `CreateAt`, a checked
three-byte creation plan, and the actual reservation, preparation, settlement, and completed-context
types. Its operational case returns `AuthorityUnavailable` once, observes the committed pending
Stop without provider entry during cold inspection, then explicitly resumes the same authority.

The checkpoint control uses the same checked transaction-plan context with an unrelated integer
field. One case preserves that field; the other increments it before the second checkpoint. Both
retain two 656-byte contexts, or 1,312 checkpoint bytes. At the second frame, the identical control
has four repeated occurrences and the distinct control has three. Their wire lengths are equal:
inline payloads are not deduplicated.

## Loading and native construction

Normal terminal loading transfers two rows. Portfolio success transfers 52,245 bytes; its domain
failure transfers 55,738 bytes. The one-retry anchored history transfers 23,963 bytes after four
frames; the five-retry history transfers 23,964 bytes after twelve frames. Both use the same Program
and produce the same final output reference. The one-byte difference is the wider sequence number.
No prior frame rows or aggregate history scan participate in either load.

Single debug-profile `Object::decode` observations were approximately 9.8 ms for the 516-byte
Portfolio input, 119 ms for the 2,329-byte anchored workflow, and 21.6 ms for the 656-byte transaction
input. These include the typed entry check and native construction. They are diagnostic observations,
not latency guarantees or a speedup comparison. The nontrivial native construction cost remains
visible even though normal loading no longer depends on history length.

## Actual rejection

The Runtime consuming regression returns an original whose string field alone is 32 MiB. Bounded
encoding rejects the complete object, preserves that native original and the separate encoding
cause, and leaves admission at sequence 1. Classification is never entered and no candidate frame
is fabricated. The same public adapter path also injects an encoding-task panic and verifies
that the native original remains in async custody without classification or append. Size reporting uses `observed_at_least`, not a fictitious exact complete length.

Declared-original native projection reapplies Values admission. A separate canary regression
returns a value rejected by the secret-marker policy: no original frame is appended, its complete
native owner remains available to the library caller, and public recording-error projection fails
with the reviewed Values rejection instead of emitting the rejected original. This covers the
failure path where the initial Values admission never succeeded.

A consuming Runtime report test commits a 16 MiB domain failure and maps it through the identity
map. The original and root each fit the object limit, but their combined terminal report exceeds
32 MiB. Its native reporting/encoding cause retains the serialization bound; no terminal candidate
is sealed. Cold inspection still returns the original at AwaitingRecovery, sequence 2. This
exercises the derived-report check before terminal append, separately from frame-count rejection.

A separate public Store exercise appended actual 16 MiB payload frames. After 31 acknowledged
frames totaling 520,101,729 bytes, the next 16,777,478-byte candidate was rejected with attempted
total 536,879,207 against the unchanged 536,870,912-byte ceiling. A subsequent bounded load retained
the exact prior head, sequence, and total. The existing complete-frame and metadata ceilings were
not increased.

The [Runtime frame-count regressions](../../crates/kernel/runtime/tests/current_state/capacity.rs)
physically append 65,536 frames through MemoryStore. They retain a real admission and a current
Runtime record, with opaque intervening frames that make no historical semantic claim. At that
head, recovery of an acknowledged original and recording of an externally returned settlement
both fail at sequence 65,537. Cold inspection retains the original AwaitingRecovery state or the
same pending EffectId respectively. The settlement adapter ran once, but interpretation did not
run and no settlement acknowledgement is claimed. These exercise the actual frame-count bound;
they do not simulate Store failure or prove that every intermediate frame was a Runtime transition.

The measurements used a temporary consuming example with a Store wrapper that retained only
successfully inserted frames for inspection. The wrapper was outside production ownership and
introduced no alternative persistence or execution route. All Rust commands used the default Nix
development shell. The managed PostgreSQL and SQLx checks exercised the same bounded Store contract.

## Deletions and replacement cost

The current core candidate has 28,019 production Rust code lines versus 26,797 at `7f71beef`:
**+1,222 lines**, including new files in both `crates` and `bin`. This counts nonblank, non-comment
lines in production `src` files, excluding separate test modules and trailing inline test modules.
Nonblank lines including comments increased by 1,232; physical lines increased by 1,138. Formatting
can change these counts without changing the design. These are the candidate's measured costs,
not an assertion that fewer source lines were achieved.

Steps 1–2 removed 630 production code lines. The inseparable step-3 replacement adds 1,852 code
lines relative to step 2. The cumulative owner breakdown is:

| Owner | Baseline code lines | Current code lines | Delta |
| --- | ---: | ---: | ---: |
| Canonical | 896 | 1,077 | +181 |
| Values | 2,433 | 2,723 | +290 |
| Program | 1,596 | 1,385 | −211 |
| Program derive | 1,344 | 1,360 | +16 |
| Capabilities | 65 | 62 | −3 |
| Journal | 946 | 175 | −771 |
| Store | 572 | 584 | +12 |
| Runtime | 3,366 | 4,254 | +888 |
| Diagnostics | 758 | 776 | +18 |
| Identities | 1,016 | 1,029 | +13 |
| Application | 1,597 | 1,979 | +382 |
| CLI | 574 | 818 | +244 |
| REST | 439 | 509 | +70 |

The remaining cumulative changes are domain code −220, live adapter code +291, PostgreSQL +6,
and signing +16. The live/signing changes support the reached native callback and task boundary;
they do not close the upstream owner inventory assigned to step 4.

The physical removals include Program recovery bounds (143 code lines), domain prospective bounds
(92), Runtime's historical fold (627), and Journal lifecycle conclusions (136). Journal also loses
its history qualification and object-table machinery in its remaining file. The complete Runtime
replacement cost is included above: deleting the fold is not credited as a net Runtime reduction.

The simpler ownership model has one Runtime continuation and one local current-record validator.
Journal seals opaque frames; Store selects admission/latest/probe instead of transferring a full
prefix. Objects contain one content identity and canonical payload, with no second contract ref,
native-value cache, or object-table resolver. Immutable identity/byte sharing avoids copying those
owners when the same Object appears in current state and facts. No alternate decoder registry,
persisted internal-fault contract, or second history representation was introduced.

Native Object admission uses a separate inner result from wire parsing. Private Runtime seeds
construct the existing current-record types directly; replacing derived decoding retains native
constructor causes without a parallel wire model or registry. This adds explicit container
consumption and error-precedence handling. One native projection unwind boundary also returns a
reviewed failure with explicit payload withholding while retaining the borrowed original.

Necessary additions account for the increase: original-failure and settlement durability require
explicit intermediate phases and separate recovery/interpretation transitions; failed recording
requires native original/candidate custody and an independent probe finding; bounded fallible
encoding must preserve its actual cause; and App/transport reporting must retain the primary result
when preparation, encoding, or delivery fails. The App and binary increases include these previously
missing reporting paths. They are not evidence of a smaller overall source tree. The acceptance
review must assess these costs together with the removed responsibilities before closing step 3.

## Material uncertainties

None within the measured fixtures. This sample does not promise that every future admitted result
fits; the actual rejection paths remain required. The [core acceptance evidence](auditability-core-acceptance.md) covers the consuming requirements;
passing these measurement fixtures alone does not close the core gate.
