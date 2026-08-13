# mfm CLI

The CLI is a non-interactive transport wrapper over the fixed-tenant App. It accepts no credential
or tenant override and renders one redacted JSON/text surface.
Admission input is capped at 512 KiB before parsing and uses the same typed domain validation as
the REST transport.
