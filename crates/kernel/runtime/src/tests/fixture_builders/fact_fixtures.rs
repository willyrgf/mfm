use super::*;

pub(in crate::tests::support) fn fixture_with_first_node_fact_descriptor(
) -> (Fixture, mfm_facts::FactDescriptor, spec::FactDescriptorRef) {
    fixture_with_node_fact_descriptor(|fixture| fixture.cell_a.clone())
}

pub(in crate::tests::support) fn fixture_with_read_node_fact_descriptor(
) -> (Fixture, mfm_facts::FactDescriptor, spec::FactDescriptorRef) {
    fixture_with_node_fact_descriptor(|fixture| fixture.cell_b.clone())
}

pub(in crate::tests::support) fn fixture_with_read_node_and_other_node_fact_descriptors() -> (
    Fixture,
    mfm_facts::FactDescriptor,
    mfm_facts::FactDescriptor,
) {
    let (mut fixture, read_descriptor, _read_ref) = fixture_with_read_node_fact_descriptor();
    let other_descriptor = test_fact_descriptor_with_kind("mfm.runtime.test.other_fact");
    let other_ref =
        mfm_program::fact_descriptor_ref_for_descriptor(&other_descriptor).expect("descriptor ref");
    let mut envelope = fixture.runtime_spec.envelope().clone();
    let node = envelope
        .spec
        .nodes
        .iter_mut()
        .find(|node| node.output_cell == fixture.cell_a)
        .expect("other fixture node");
    let descriptor_id = node.descriptor_id.clone();
    node.fact_descriptor_allowlist = vec![other_ref.clone()];
    let state_descriptor = envelope
        .spec
        .descriptor_identities
        .iter_mut()
        .find_map(|identity| match identity {
            spec::DescriptorIdentity::State(state) if state.descriptor_id == descriptor_id => {
                Some(state)
            }
            _ => None,
        })
        .expect("other fixture state descriptor");
    state_descriptor.emitted_fact_descriptors = vec![other_ref];
    let envelope =
        spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("descriptor rehash");
    fixture.runtime_spec =
        CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("descriptor runtime spec");
    refresh_fixture_run_id(&mut fixture);
    (fixture, read_descriptor, other_descriptor)
}

pub(in crate::tests::support) fn fixture_with_node_fact_descriptor(
    select_output_cell: impl FnOnce(&Fixture) -> CellId,
) -> (Fixture, mfm_facts::FactDescriptor, spec::FactDescriptorRef) {
    let mut fixture = fixture();
    let output_cell = select_output_cell(&fixture);
    let descriptor =
        <RuntimeTestFact as mfm_program::MfmFactType>::descriptor().expect("fact descriptor");
    let descriptor_ref =
        mfm_program::fact_descriptor_ref_for_descriptor(&descriptor).expect("descriptor ref");
    let mut envelope = fixture.runtime_spec.envelope().clone();
    let node = envelope
        .spec
        .nodes
        .iter_mut()
        .find(|node| node.output_cell == output_cell)
        .expect("fixture node");
    let descriptor_id = node.descriptor_id.clone();
    node.fact_descriptor_allowlist = vec![descriptor_ref.clone()];
    let state_descriptor = envelope
        .spec
        .descriptor_identities
        .iter_mut()
        .find_map(|identity| match identity {
            spec::DescriptorIdentity::State(state) if state.descriptor_id == descriptor_id => {
                Some(state)
            }
            _ => None,
        })
        .expect("fixture state descriptor");
    state_descriptor.emitted_fact_descriptors = vec![descriptor_ref.clone()];
    let envelope =
        spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("descriptor rehash");
    fixture.runtime_spec =
        CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("descriptor runtime spec");
    refresh_fixture_run_id(&mut fixture);
    (fixture, descriptor, descriptor_ref)
}
