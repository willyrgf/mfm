# mfm-capabilities

`ReadCapabilityContract` names semantic Intent and Evidence. `EffectCapabilityContract` names
semantic Command and Evidence. Their binding checks receive the authoritative native evidence
reference as well as the semantic request; Effect checks also receive the exact command reference
and EffectId. Program's semantic capability identity commits mode and those semantic contracts.

`ReadImplementation<C>` and `EffectImplementation<C>` own the selected public Binding, native
request, native evidence and OperationalError contracts. Their pure translation/projection methods
validate native correspondence and expose a semantic view of the exact original Object. They perform
no IO, configuration lookup, classification or execution scheduling. Native hooks return the
Capabilities-owned `CallbackFailure`: Decode, Execute or Encode with the exact invocation diagnostic.
Nested codecs use synchronous `codec::decode`/`codec::encode` scopes; codec panics retain their phase
without their payload. Semantic mismatches use Execute. Program preserves this phase, while Runtime
adds the originating operation and run context. Ordinary State/construction boundaries retain nested
codec phase, cause and size through `into_diagnostic`; no native-original custody is introduced.

`ReadAdapter<Intent, Evidence, Failure>` and `EffectAdapter<Command, Evidence, Failure>` expose
explicit async IO. Invocation receives separate semantic and native request references. Effect also
receives the retained EffectId. Program binds supplied live resources before execution; Runtime owns
acknowledgement and continuation.

`AdapterError<E>` separates an exact typed operational original from an invocation-only
`InvocationDiagnostic` local failure. Debug and Display redact the operational payload;
selected dependency diagnostics follow the [owner trust contract](../../../docs/design.md).
Runtime retains acknowledged originals before recovery or public reporting. Checked capability
identity failures retain their concrete grammar rejection through `CapabilityError::Identity`.

`EffectAdapterOutcome<Evidence>` distinguishes Pending from Settled evidence. Pending does not
fabricate evidence, imply nonacceptance, replace command authority or acknowledge a conclusion.
This adapter outcome is separate from deterministic State success/failure.

The crate owns no State outcome, scheduler, provider API, transport, signer or mutation authority.
