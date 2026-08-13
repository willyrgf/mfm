# mfm-replay

Replay qualifies complete retained frames and canonical v5 portable streams without Runtime,
State, adapter, signer, or provider callbacks. The stream carries its format and Store/tenant
identity; callers may verify its content reference before frame parsing.
Replay consumes only cloneable callback-free `QualifiedRun` evidence and has no dependency on
`SelectedRun` or a Store mutation port. Public Journal DTO construction is wire structure, never
append authority.
