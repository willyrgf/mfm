use super::*;

pub(in crate::tests::support) fn fixture_with_fact_descriptor(
) -> (Fixture, mfm_facts::FactDescriptor, spec::FactDescriptorRef) {
    let fixture = certified_base_fixture(true);
    let descriptor =
        <RuntimeTestFact as mfm_facts::MfmFactType>::descriptor().expect("fact descriptor");
    let descriptor_ref =
        mfm_program::fact_descriptor_ref_for_descriptor(&descriptor).expect("descriptor ref");

    assert!(
        fixture
            .runtime_spec
            .spec()
            .nodes
            .iter()
            .any(|node| node.fact_descriptor_allowlist.contains(&descriptor_ref)),
        "certified runtime must retain the fact-emitting node"
    );

    (fixture, descriptor, descriptor_ref)
}
