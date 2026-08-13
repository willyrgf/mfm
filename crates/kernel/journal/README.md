# mfm-journal

Journal owns strict canonical frames and exactly three run record families: `RunAdmitted`,
`StatePrepared`, and `StateConcluded`. Store owns reduction and append semantics.
Successful conclusions may retain a coordinate-free fact-proposal object in their append closure;
Store later assigns any publication coordinate atomically with the conclusion append.
