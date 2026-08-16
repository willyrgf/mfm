# Run execution

One run advances through a sequential cumulative context:

```text
C0 -> RunAdmitted
  -> Pure: evaluate(Cn) -> StateConcluded
  -> Read/Effect: prepare(Cn) -> StatePrepared
                  -> direct-new CommittedCall
                  -> adapter evidence -> StateConcluded
  -> Cn+1 or terminal root result
```

Admission is the only genesis. Store selects at most one actionable occurrence from a valid prefix.
Trusted composition first publishes the domain-owned Portfolio configuration value and gives each
execution mutation port its own same-opening resolved head. Admission accepts a
catalog-qualified typed value and that Store-issued head; Store derives the genesis frame, sequence,
and physical append identity. It records the selected head's global sequence and typed content
identity; absent, foreign-opening, wrong-type, or fabricated heads fail before the genesis append.
Pure never creates a preparation. Read and Effect cannot enter a provider until their exact
preparation append is newly committed. A found, stale, invalid, terminal, or ambiguous append
creates no call.

For a prior-fact capability, preparation carries only the request projected from the canonical
intent. Store checks the request against the admitted source manifest, reads one bounded fact
publication snapshot, scans the published proposal sets, and appends the resulting selection object
with `StatePrepared`. The direct-new continuation carries that Store-fixed typed selection and
frontier; callers cannot provide or refresh it. Each selected fact is paired with Store-authored
producer Program/run/record/head provenance, and load/append qualification verifies the producer
history and exact fact content. The frontier stream is bound to the Store scope, writer epoch,
and tenant, so a copied selection cannot cross partitions or epochs.

Restore is an explicit deployment cutover: PostgreSQL persists the active scope/epoch identity,
rotates it to the fresh restore identity, and rejects old-identity handles before they can resume
or append a run.

Successful State conclusions carry a coordinate-free proposal set. Store assigns the next tenant
fact-publication coordinate only when the proposal set is non-empty and appends the conclusion and
publication atomically. A fact-frontier race returns the same pending conclusion owner with its
coordinate cleared and a fresh Store-owned physical append id; retrying rebinds that coordinate
without re-running State, adapter, or fact-selection logic. PostgreSQL materializes and locks an
empty fact-head row before checking the first publication, so two independent first publishers
produce one winner and one frontier-change result rather than a unique-key ambiguity.

Conclusion recovery classifies a stale same-run head before Runtime settles it: identical semantic
records resume from qualified history, a superseded Access preparation returns `NoLongerSelected`,
a different conclusion returns `Conflict`, and a malformed or impossible prefix returns
`InvalidHistory`. None of these branches re-enters State, adapters, interpretation, or fact scans.

If the backend acknowledges a conclusion append ambiguously, Runtime retains the same pending
owner. Resolution retries the same append identity, accepts `Found` for the exact retained frame,
and never republishes its fact proposal set or re-enters Pure, Access, adapter, or interpretation
logic.

The worker retains the latest catalog-qualified typed context while hot. A conclusion is canonically
encoded once for the append, then the already-typed successor is handed to the next State. Cold
resume asks the non-Clone mutation port for one affine `SelectedRun`, folds the complete bounded
prefix once, and checks every content/contract link without invoking State or adapter callbacks.
Hot preparation and conclusion consume that owner; a direct append returns the next selected owner
without reloading or refolding history. Converting it to cloneable `QualifiedRun` evidence ends
mutation authority.

Admission, preparation, conclusion, and acknowledgement uncertainty return exhaustive Runtime
outcomes. `SuspendedRun` retains the exact owner across a retryable physical boundary; no generic
drop or status-only success path creates execution authority.

Effect has one total attempt and parks after an unresolved entry. Read may replace a selected
preparation only within its fixed total budget, preserving exact input, intent, and binding. A late
Read result cannot settle a superseded occurrence.

The recovery matrix is intentionally owner-based:

| Outcome | Re-execution | Owner/result |
| --- | ---: | --- |
| Same conclusion, different physical id | zero | recorded qualified history |
| Access preparation superseded | zero | latest history and `NoLongerSelected` |
| Different same-occurrence conclusion | zero | qualified conflict |
| Permanent Store rejection | zero | distinct owner-bearing rejection |
| Unknown acknowledgement | zero | exact pending owner, resolved by physical id |

Every successful nonterminal State returns the complete next domain context. Failure is fail-fast;
recovery context exists only when the domain failure value explicitly contains it. Match consumes
one exact closed-sum payload and continuing arms converge on one contract.
