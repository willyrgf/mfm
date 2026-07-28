# mfm-facts

Pure fact authoring and selection semantics for the typed kernel.

The crate owns:

- exact descriptor projections and bounded canonical subject values;
- explicit producer-independent subject/response `ProposedFactValue` material,
  slot-indexed same-run `FactProposal`, and ordered, duplicate-free `FactSet`
  with nondecreasing fact-slot groups;
- the annex-backed `FactSelectionQuery` and `FactSelectionRequest`;
- exact subject predicates, publication ordering, logical-identity tie-breaks,
  limits, and a bounded pure top-K accumulator; and
- the frozen domain-separated fact-query digest.

It owns no store, scan session, journal coordinate, retained-object authority,
response, completeness proof, public fact projection, receipt, or auxiliary
evidence bag. The journal owns persisted fact references and response
envelopes. The store privately scans the authoritative tenant prefix and may
use the pure evaluator here, but no value in this crate proves completeness.

`ProposedFactValue` carries exact canonical bytes plus the complete
producer-independent retained-value contract, including its evidence contract.
It is not a `ValueRef` and grants no object authority. A `FactProposal`
explicitly selects its process-only certified fact-slot ordinal and supplies
distinct subject and response positions. Store-owned materialization validates
the complete group count and both values against that slot, mints their
producer-bound references, and alone constructs the fixed journal
`FactClaimEnvelope`. State code cannot author or substitute that envelope.

`FactSelectionRequest::from_canonical_json` and
`FactSelectionQuery::from_canonical_json` strictly decode exact canonical bytes
with the embedded recoverability annex. Schema identities and query digests
come from that annex; there is no Serde-derived or compatibility wire format.
