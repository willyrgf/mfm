# mfm-journal

Journal owns the exact `mfm.run.frame.v2` encoder and qualifier. Frames contain admission, fused
Pure conclusion, fused Read intent/evidence/outcome conclusion, or separate Effect prepare and
conclusion records plus an exact sorted frame-local object closure. A prepare may end a valid
prefix; otherwise only its adjacent conclusion can follow. Recursive heads hash exact bytes with
`content:sha256-v1`.

`EncodedRunFrame`, opaque `StoredRunBytes`, qualified `JournalHistory`, borrowed record/object views,
four format constants, and `frame_head_digest` are the complete surface. There is no raw parser,
open wire DTO, independent record digest, or portable codec. Journal checks Effect adjacency but
does not derive an `EffectId`, interpret a command, or select a Program declaration.
