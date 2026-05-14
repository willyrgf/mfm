# mfm-machine-derive

Reserved proc-macro integration point for the `mfm-machine` runtime.

This crate intentionally exposes no public derive or attribute macros today.
It preserves the package boundary where future compile-time helpers for common
machine/state patterns will live, without adding placeholder macro APIs.
