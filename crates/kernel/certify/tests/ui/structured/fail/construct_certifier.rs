use mfm_certify::structured::{AdmissionVerifier, EntryPointCertifier};

fn value<T>() -> T {
    panic!()
}

fn certifier<'a>() -> EntryPointCertifier<'a> {
    EntryPointCertifier {
        entry: value(),
        registry: value(),
    }
}

fn verifier<'a>() -> AdmissionVerifier<'a> {
    AdmissionVerifier {
        entry: value(),
        registry: value(),
    }
}

fn main() {}
