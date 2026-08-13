# mfm-store

Store validates complete frames, reduces one sequential prefix, selects one preparation, enforces
occurrence-level conclusion uniqueness, and returns exact-head append owners. Public mutation is
split between sole-genesis admission and Store-owned qualified conclusions; there is no generic
frame append. Qualified runs, reductions, configuration snapshots, fact continuations, and append
owners are tied to the exact Store opening; matching persisted identity fields do not permit
transposition. It cannot invoke Runtime or provider code.
