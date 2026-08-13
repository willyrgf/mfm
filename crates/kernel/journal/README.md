# mfm-journal

Journal owns strict canonical wire syntax and validation for exactly three public checked DTO
families: `RunAdmitted`, `StatePrepared`, and `StateConcluded`, wrapped by `RunRecord` and
`RunFrame`. Constructing them proves bounded canonical structure, not semantic mutation authority.
They remain available to wire tests, replay/export, storage decoding, and backend conformance.
Store alone owns reduction, journal-coordinate assignment, and semantic append authority.
Successful conclusions may retain a coordinate-free fact-proposal object in their append closure;
Store later assigns any publication coordinate atomically with the conclusion append.
