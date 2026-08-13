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
Pure never creates a preparation. Read and Effect cannot enter a provider until their exact
preparation append is newly committed. A found, stale, invalid, terminal, or ambiguous append
creates no call.

The worker retains the latest catalog-qualified typed context while hot. A conclusion is canonically
encoded once for the append, then the already-typed successor is handed to the next State. Cold
resume folds the complete bounded prefix and checks every content/contract link without invoking
State or adapter callbacks.

Admission, preparation, conclusion, and acknowledgement uncertainty return exhaustive Runtime
outcomes. `SuspendedRun` retains the exact owner across a retryable physical boundary; no generic
drop or status-only success path creates execution authority.

`EntryOnce` has one total attempt and parks after an unresolved entry. Read and a proven absorbing
Effect may replace a selected preparation only within their fixed total budget, preserving exact
input, intent, binding, domain, and absorption identity. A late result cannot settle a superseded
occurrence.

Every successful nonterminal State returns the complete next domain context. Failure is fail-fast;
recovery context exists only when the domain failure value explicitly contains it. Match consumes
one exact closed-sum payload and continuing arms converge on one contract.
