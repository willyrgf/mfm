use mfm_certify::structured::EntryPointCertifier;
use mfm_spec::structured::{AuthoredStructuredProgram, StructuredExpansionProfile};

fn weaken(
    certifier: &EntryPointCertifier<'_>,
    program: AuthoredStructuredProgram,
    profile: StructuredExpansionProfile,
) {
    let _ = certifier.certify(program, profile);
}

fn main() {}
