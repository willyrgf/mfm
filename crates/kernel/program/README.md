# mfm-program

Program owns the one checked canonical `mfm-program-document@2` graph and the typed `State`,
`PureState`, `ReadState`, `ProposedStateOutcome`, `ReadPreparationError`, and `Never` contracts.

Index zero is root. Declarations are State or closed-sum Match. State successors are optional forward
`u16` indices; absence means that branch's exact root contract. Read declarations retain capability,
intent, evidence, and binding refs. Construction/decoding rejects v1, unknown fields, unreachable or
backward edges, contract discontinuity, invalid Never placement, and fixed bounds.

Program is content addressed and has no catalog, registry, erased value, runtime implementation,
configuration contract, or second wire DTO.
