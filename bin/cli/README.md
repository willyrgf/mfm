# mfm CLI

The CLI is a non-interactive transport wrapper over the fixed-tenant App. It accepts no credential
or tenant override and renders one redacted JSON/text surface.
Admission input is capped at 512 KiB before parsing and uses the same typed domain validation as
the REST transport.

Startup explicitly opens the demo Store, publishes secret-free typed Portfolio and EVM
configuration revisions, consumes the opening into explicit mutation/read/configuration/audit
ports, and constructs App with those ports and resolved heads. The CLI exposes no public
configuration write surface.
