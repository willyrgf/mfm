# mfm-store

Store validates complete frames, reduces one sequential prefix, selects one preparation, enforces
occurrence-level conclusion uniqueness, and returns exact-head append owners. Public mutation is
split between sole-genesis admission and Store-owned qualified conclusions; there is no generic
frame append. It scans published fact proposal sets into a typed preparation-bound continuation,
and assigns conclusion publication coordinates atomically with the fact head. Selections carry
scope/epoch/tenant-bound frontier identity and aligned producer Program/run/record/head evidence;
Store validates that evidence against the retained producer frame and exact fact value. A
fact-frontier race returns the same semantic owner with a fresh physical append identity for
coordinate-only rebinding. Qualified runs, reductions,
configuration snapshots, fact continuations, and append owners are tied to the exact Store
opening; matching persisted identity fields do not permit transposition. It cannot invoke Runtime
or provider code. Acknowledgement recovery retries the same physical append identity and returns
the retained frame on `Found`, so fact publication is not duplicated. A stale same-run conclusion
is classified as identical, superseded Access, conflict, or invalid history before Runtime can
advance it.
