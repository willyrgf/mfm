use mfm_capabilities::{
    CapabilityKind, CapabilitySetFor, CapabilitySpec, CapabilityVersion,
    ExternalMutationAuthorityRole, ReadExternalRole, Result, SupportRole,
};

pub struct ReadCap;
pub struct SupportCap;
pub struct MutationCap;

impl CapabilitySpec for ReadCap {
    type Role = ReadExternalRole;

    fn kind() -> Result<CapabilityKind> {
        unimplemented!()
    }

    fn version() -> Result<CapabilityVersion> {
        unimplemented!()
    }

    fn name() -> &'static str {
        "read_cap"
    }
}

impl CapabilitySpec for SupportCap {
    type Role = SupportRole;

    fn kind() -> Result<CapabilityKind> {
        unimplemented!()
    }

    fn version() -> Result<CapabilityVersion> {
        unimplemented!()
    }

    fn name() -> &'static str {
        "support_cap"
    }
}

impl CapabilitySpec for MutationCap {
    type Role = ExternalMutationAuthorityRole;

    fn kind() -> Result<CapabilityKind> {
        unimplemented!()
    }

    fn version() -> Result<CapabilityVersion> {
        unimplemented!()
    }

    fn name() -> &'static str {
        "mutation_cap"
    }
}

pub fn assert_capability_set<E, C>()
where
    E: mfm_capabilities::EffectSpec,
    C: CapabilitySetFor<E>,
{
}
