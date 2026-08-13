# mfm-store

Store validates complete frames, reduces one sequential prefix, selects one preparation, enforces
occurrence-level conclusion uniqueness, and returns exact-head append owners. Public mutation is
split between sole-genesis admission and Store-owned qualified conclusions; there is no generic
frame append. It cannot invoke Runtime or provider code.
