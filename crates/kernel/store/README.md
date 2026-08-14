# mfm-store

Store validates complete frames, reduces one sequential prefix, selects one action, enforces
occurrence-level conclusion uniqueness, and returns exact-head append owners. Cloneable
`QualifiedRun` is callback-free evidence; affine `SelectedRun` is the sole Store-selected run
mutation owner. Public mutation consumes catalog-qualified admission, intent, evidence, and
outcome values plus concrete fact values; Store validates exact capability/evidence association,
derives every journal ref, object closure, binding identity, and coordinate, and there is no generic
frame append or evidence-promotion API. It scans published fact proposal sets into a typed
preparation-bound continuation,
and assigns conclusion publication coordinates atomically with the fact head. Selections carry
scope/epoch/tenant-bound frontier identity and aligned producer Program/run/record/head evidence;
Store validates that evidence against the retained producer frame and exact fact value. A
fact-frontier race returns the same semantic owner with a fresh physical append identity for
coordinate-only rebinding. Selected runs, resolved typed configurations and their
erased heads, fact continuations, and append owners are tied to the exact Store opening; matching
persisted identity fields do not permit transposition. It cannot invoke Runtime or provider code.
Acknowledgement recovery retries the same physical append identity and returns the retained frame
on `Found`, so fact publication is not duplicated. Configuration `Found` returns its retained row
for one typed re-ingress. Local `ValidatedConfig<C>` publication does no decode; external and
retained bytes are strict canonical, float-free, secret-free, exact-schema input. Admissions
require the same-opening resolved head and persist its sequence and typed content ref. A stale
same-run conclusion is classified as identical, superseded Access, conflict, or invalid history
before Runtime can advance it. An admission closure also contains exactly one canonical
`mfm.program` object matching `RunAdmitted.program_ref` and entry point. Store selects by run id,
strictly ingresses that retained document under its catalog, and never accepts a caller-supplied
or reconstructed Program during cold selection.
