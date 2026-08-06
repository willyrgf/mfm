# mfm-facts

Pure fact authoring and selection semantics for the typed kernel.

The crate owns:

- exact descriptor projections and bounded canonical subject values (boolean, string, or
  unsigned-integer scalars under the unsigned-native recoverability contract);
- explicit producer-independent subject/response `ProposedFactValue` material,
  slot-indexed same-run `FactProposal`, and ordered, duplicate-free `FactSet`
  with nondecreasing fact-slot groups;
- the annex-backed `FactSelectionQuery` and `FactSelectionRequest`, including the admitted source
  reference, fixed selector, completeness mode, and five total scan bounds;
- the ordinary Read's typed canonical-response wrapper and closed redaction-safe failure codes;
- exact subject predicates, publication ordering, logical-identity tie-breaks,
  limits, and a bounded pure top-K accumulator; and
- the frozen domain-separated fact-query digest.

It owns no store, scan session, journal coordinate, retained-object authority,
completeness proof, public fact projection, receipt, or auxiliary evidence bag.
The journal owns persisted fact references and the semantic response/attestation
envelope. The store privately scans the authoritative tenant prefix and may use
the pure evaluator here, but no value in this crate proves completeness.

`ProposedFactValue` carries exact canonical bytes plus the complete
producer-independent retained-value contract, including its evidence contract.
It is not a `ValueRef` and grants no object authority. A `FactProposal`
explicitly selects its process-only certified fact-slot ordinal and supplies
distinct subject and response positions. Store-owned materialization validates
the complete group count and both values against that slot, mints their
producer-bound references, and alone constructs the fixed journal
structured fact-claim object. State code cannot author or substitute that object.

`FactSelectionRequest::from_canonical_json` and
`FactSelectionQuery::from_canonical_json` strictly decode exact canonical bytes
with the embedded recoverability annex. Schema identities and query digests
come from that annex. The MFM Read value carries those exact inner bytes in one
base64url field whose deserializer revalidates them; there is no compatibility
wire format.
