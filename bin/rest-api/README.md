# mfm REST API

The REST binary is a transport wrapper over a trusted fixed-tenant App. Requests contain one
entry-point id and one singular input; authentication and tenant policy are outside this facade.
Admission bodies are capped at 512 KiB and use strict duplicate/unknown-field rejection before
domain validation. The transport exposes callback-free read, drive, replay, trace, audit, and
portable-export routes; responses expose only redacted public error codes.
