use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CellInfo {
    pub(super) producer: spec::CellProducer,
    pub(super) semantic_type_id: SemanticTypeId,
    pub(super) schema_id: SchemaId,
    pub(super) value_lineage: spec::ValueLineageRef,
    pub(super) context: spec::CellContextSpec,
}

pub(super) struct DraftLowerer<'a> {
    draft: &'a program::TypedProgramDraft,
    cells: Vec<spec::CellSpec>,
    cell_info: BTreeMap<String, CellInfo>,
    config_refs: BTreeMap<String, spec::ConfigRef>,
    descriptor_identities: BTreeMap<String, spec::DescriptorIdentity>,
    value_lineages: BTreeMap<String, spec::ValueLineage>,
    nodes: Vec<spec::NodeSpec>,
}

struct FrameworkLifecycleNode {
    node_id: NodeId,
    stable_key: spec::StableAuthorKey,
    config_ref: spec::ConfigRef,
    input_binding: spec::InputBindingSpec,
    descriptor: spec::StateDescriptorIdentity,
    input_cells: Vec<CellId>,
    framework: spec::FrameworkNodeSpec,
    planning_lineage: spec::PlanningLineage,
}

impl<'a> DraftLowerer<'a> {
    pub(super) fn new(draft: &'a program::TypedProgramDraft) -> Result<Self> {
        Ok(Self {
            draft,
            cells: Vec::new(),
            cell_info: BTreeMap::new(),
            config_refs: BTreeMap::new(),
            descriptor_identities: BTreeMap::new(),
            value_lineages: BTreeMap::new(),
            nodes: Vec::new(),
        })
    }

    pub(super) fn lower(&mut self) -> Result<spec::TypedExecutionSpec> {
        let scopes = self.lower_scopes()?;
        let seeds = self.lower_seeds()?;
        self.register_planned_cells()?;
        self.lower_state_nodes()?;
        let remediations = self.lower_remediation_nodes()?;
        self.lower_bridge_nodes()?;
        let public_outputs = self.lower_public_outputs()?;
        let planning_lineage = self.lower_operation_lineage()?;

        spec::TypedExecutionSpec::new(spec::TypedExecutionSpecParts {
            authoring: self.authoring_provenance()?,
            saga: lower_saga_policy(self.draft.saga_policy()),
            contexts: self.draft.contexts().to_vec(),
            scopes,
            seeds,
            descriptor_identities: self.descriptor_identities.values().cloned().collect(),
            config_refs: self.config_refs.values().cloned().collect(),
            nodes: self.nodes.clone(),
            remediations,
            cells: self.cells.clone(),
            value_lineages: self.value_lineages.values().cloned().collect(),
            planning_lineage,
            public_outputs,
        })
        .map_err(|error| CertifyError::Spec(error.to_string()))
    }

    fn lower_scopes(&self) -> Result<Vec<spec::ScopeSpec>> {
        self.draft
            .scopes()
            .iter()
            .map(|scope| {
                Ok(spec::ScopeSpec {
                    scope_id: scope.scope_id.clone(),
                    parent_scope_id: scope.parent_scope_id.clone(),
                    stable_key: stable_author_key(scope.key.as_str())?,
                    planning_lineage: lower_planning_lineage(&scope.planning_lineage),
                })
            })
            .collect()
    }

    fn lower_seeds(&mut self) -> Result<Vec<spec::SeedSpec>> {
        let mut lowered = Vec::new();
        for seed in self.draft.seeds() {
            let lineage = spec::ValueLineage {
                lineage_ref: lineage_ref(seed.value_lineage.digest()),
                scope_id: seed.scope_id.clone(),
                producer: spec::CellProducer::Seed(seed.seed_id.clone()),
                input_cells: Vec::new(),
                config_ref_digest: None,
                planning_lineage: empty_planning_lineage()?,
                domain_keys: Vec::new(),
                transform_policy: spec::LineageTransformPolicy::Source,
            };
            self.insert_value_lineage(lineage)?;
            self.insert_cell(spec::CellSpec {
                cell_id: seed.cell_id.clone(),
                producer: spec::CellProducer::Seed(seed.seed_id.clone()),
                scope_id: seed.scope_id.clone(),
                semantic_type_id: seed.semantic_type_id.clone(),
                schema_id: seed.schema_id.clone(),
                value_lineage: lineage_ref(seed.value_lineage.digest()),
                terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
                storage_policy: spec::StoragePolicy::ContentAddressed,
                redaction_policy: spec::RedactionPolicy::Public,
                context: spec::CellContextSpec::no_context(),
            })?;
            lowered.push(spec::SeedSpec {
                seed_id: seed.seed_id.clone(),
                seed_key: stable_author_key(seed.key.as_str())?,
                cell_id: seed.cell_id.clone(),
                scope_id: seed.scope_id.clone(),
                semantic_type_id: seed.semantic_type_id.clone(),
                schema_id: seed.schema_id.clone(),
                required_digest: Some(seed.content_digest.clone()),
            });
        }
        Ok(lowered)
    }

    fn register_planned_cells(&mut self) -> Result<()> {
        for node in self
            .draft
            .state_nodes()
            .iter()
            .chain(self.draft.remediation_nodes().values())
        {
            self.register_planned_cell_info(
                CellInfo {
                    producer: spec::CellProducer::Node(node.node_id.clone()),
                    semantic_type_id: node.output_semantic_type_id.clone(),
                    schema_id: node.output_schema_id.clone(),
                    value_lineage: lineage_ref(node.output_value_lineage.digest()),
                    context: node.output_context.clone(),
                },
                &node.output_cell_id,
            )?;
            if let Some(verify) = &node.side_effect_verify {
                self.register_planned_cell_info(
                    CellInfo {
                        producer: spec::CellProducer::Node(verify.node_id.clone()),
                        semantic_type_id: node.output_semantic_type_id.clone(),
                        schema_id: node.output_schema_id.clone(),
                        value_lineage: lineage_ref(verify.output_value_lineage.digest()),
                        context: verify.output_context.clone(),
                    },
                    &verify.output_cell_id,
                )?;
            }
        }
        for bridge in self.draft.bridge_nodes() {
            self.register_planned_cell_info(
                CellInfo {
                    producer: spec::CellProducer::Node(bridge.node_id.clone()),
                    semantic_type_id: bridge.semantic_type_id.clone(),
                    schema_id: bridge.schema_id.clone(),
                    value_lineage: lineage_ref(bridge.target_value_lineage.digest()),
                    context: bridge.context.clone(),
                },
                &bridge.target_cell_id,
            )?;
        }
        Ok(())
    }

    fn lower_state_nodes(&mut self) -> Result<()> {
        for node in self.draft.state_nodes() {
            let lowered = self.lower_state_node(node)?;
            let verify = self.lower_side_effect_verify_if_present(node, &lowered, "forward")?;
            self.nodes.push(lowered.clone());
            if let Some(verify) = verify {
                self.nodes.push(verify);
            }
        }
        Ok(())
    }

    fn lower_remediation_nodes(&mut self) -> Result<BTreeMap<NodeId, spec::NodeSpec>> {
        let mut remediations = BTreeMap::new();
        for (forward_node_id, node) in self.draft.remediation_nodes() {
            let lowered = self.lower_state_node(node)?;
            if let Some(mut verify) =
                self.lower_side_effect_verify_if_present(node, &lowered, "remediation")?
            {
                verify.deterministic_predecessors.clear();
                self.nodes.push(verify);
            }
            remediations.insert(forward_node_id.clone(), lowered);
        }
        Ok(remediations)
    }

    fn lower_side_effect_verify_if_present(
        &mut self,
        node: &program::StateNodeSpec,
        lowered: &spec::NodeSpec,
        node_kind: &str,
    ) -> Result<Option<spec::NodeSpec>> {
        if lowered.side_effect.is_none() {
            return Ok(None);
        }
        let verify = node.side_effect_verify.as_ref().ok_or_else(|| {
            problem(
                ProblemClass::InvalidTopology,
                format!(
                    "{node_kind} side-effect node {} is missing verify pair",
                    node.node_id
                ),
            )
        })?;
        self.lower_side_effect_verify_node(node, lowered, verify)
            .map(Some)
    }

    fn lower_state_node(&mut self, node: &program::StateNodeSpec) -> Result<spec::NodeSpec> {
        let config_ref = self.config_ref(&node.config)?;
        let input_bindings = lower_input_binding(&node.input)?;
        let input_cells = collect_input_cells(&input_bindings.root);
        let planning_lineage = lower_planning_lineage(&node.planning_lineage);
        let domain_keys = node
            .output_domain_keys
            .iter()
            .map(lower_domain_key_ref)
            .collect::<Vec<_>>();
        let output_lineage = spec::ValueLineage {
            lineage_ref: lineage_ref(node.output_value_lineage.digest()),
            scope_id: node.scope_id.clone(),
            producer: spec::CellProducer::Node(node.node_id.clone()),
            input_cells: sorted_cell_ids(input_cells.clone()),
            config_ref_digest: Some(node.config.config_ref_digest.clone()),
            planning_lineage: planning_lineage.clone(),
            domain_keys,
            transform_policy: spec::LineageTransformPolicy::StateOutput,
        };
        self.insert_value_lineage(output_lineage)?;
        let terminal_policy = if node.side_effect_verify.is_some() {
            spec::CellTerminalPolicy::MaybeSkipped
        } else {
            spec::CellTerminalPolicy::ProducedOnly
        };
        self.insert_cell(spec::CellSpec {
            cell_id: node.output_cell_id.clone(),
            producer: spec::CellProducer::Node(node.node_id.clone()),
            scope_id: node.scope_id.clone(),
            semantic_type_id: node.output_semantic_type_id.clone(),
            schema_id: node.output_schema_id.clone(),
            value_lineage: lineage_ref(node.output_value_lineage.digest()),
            terminal_policy,
            storage_policy: spec::StoragePolicy::ContentAddressed,
            redaction_policy: spec::RedactionPolicy::Public,
            context: node.output_context.clone(),
        })?;
        self.insert_descriptor(spec::DescriptorIdentity::State(Box::new(
            state_descriptor_identity_from_program(node)?,
        )))?;
        let side_effect = match (
            node.runner,
            node.effect_contract_digest.as_ref(),
            node.side_effect_resource_claim.as_ref(),
            node.side_effect_verification.as_ref(),
        ) {
            (
                program::RunnerKind::ApplySideEffect,
                Some(digest),
                Some(resource_claim),
                Some(verification),
            ) => Some(spec::SideEffectContractSpec {
                contract_digest: digest.clone(),
                resource_claim: resource_claim.clone(),
                verification: verification.clone(),
            }),
            (program::RunnerKind::ApplySideEffect, _, _, _) => {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!(
                        "side-effect node {} is missing its effect contract, resource claim, or verification policy",
                        node.node_id
                    ),
                ));
            }
            (_, _, None, None) => None,
            (_, _, Some(_), _) | (_, _, _, Some(_)) => {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!(
                        "non-side-effect node {} carries side-effect policy",
                        node.node_id
                    ),
                ));
            }
        };
        Ok(spec::NodeSpec {
            node_id: node.node_id.clone(),
            stable_key: stable_author_key(node.key.as_str())?,
            scope_id: node.scope_id.clone(),
            state_kind: node.state_kind.clone(),
            state_version: node.state_version.clone(),
            descriptor_id: node.state_descriptor_id.clone(),
            context: node.context.clone(),
            config_ref,
            input_bindings,
            output_cell: node.output_cell_id.clone(),
            effect_kind: node.effect_kind.clone(),
            capability_bindings: node.capability_bindings.clone(),
            adapter_bindings: node
                .adapter_bindings
                .iter()
                .map(|binding| spec::AdapterBinding {
                    adapter_kind: binding.adapter_kind.clone(),
                    adapter_version: binding.adapter_version.clone(),
                    binding_digest: None,
                })
                .collect(),
            fact_descriptor_allowlist: node.fact_descriptor_allowlist.clone(),
            side_effect,
            framework: None,
            planning_lineage,
            deterministic_predecessors: self.predecessors_for_inputs(&input_cells)?,
        })
    }

    fn lower_side_effect_verify_node(
        &mut self,
        source: &program::StateNodeSpec,
        submit: &spec::NodeSpec,
        verify: &program::SideEffectVerifyDraftSpec,
    ) -> Result<spec::NodeSpec> {
        let submit_contract = submit.side_effect.as_ref().ok_or_else(|| {
            problem(
                ProblemClass::InvalidTopology,
                format!("submit node {} is not a side-effect node", submit.node_id),
            )
        })?;
        let expected_pair =
            spec::side_effect_pair_id(&submit.node_id, &submit.output_cell, submit_contract)
                .map_err(|error| CertifyError::Spec(error.to_string()))?;
        if verify.pair_id != expected_pair {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!(
                    "verify pair for submit node {} is not stable-id derived",
                    submit.node_id
                ),
            ));
        }
        let expected_verify_node =
            spec::side_effect_verify_node_id(&submit.node_id, &verify.pair_id)
                .map_err(|error| CertifyError::Spec(error.to_string()))?;
        if verify.node_id != expected_verify_node {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!(
                    "verify node for submit node {} is not stable-id derived",
                    submit.node_id
                ),
            ));
        }
        let config_ref = framework_config_ref("side_effect_verify", &verify.node_id)?;
        self.insert_config_ref(config_ref.clone())?;
        let submit_output_cell = self
            .cells
            .iter()
            .find(|cell| cell.cell_id == submit.output_cell)
            .cloned()
            .ok_or_else(|| {
                problem(
                    ProblemClass::InvalidTopology,
                    format!("submit node {} output cell is missing", submit.node_id),
                )
            })?;
        let input_binding = spec::framework_lifecycle_maybe_skipped_cell_input_binding(
            "side_effect_verify",
            "submit_output",
            &submit_output_cell,
        )
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let node_context = node_context_from_cell_context(&submit_output_cell.context);
        let descriptor = framework_side_effect_verify_descriptor(
            &source.output_schema_id,
            &source.output_semantic_type_id,
            &config_ref.schema_id,
            &input_binding.input_schema_id,
            state_context_descriptor_from_cell_context(
                &submit_output_cell.context,
                self.draft.contexts(),
            )?,
        )?;
        self.insert_descriptor(spec::DescriptorIdentity::State(Box::new(
            descriptor.clone(),
        )))?;
        let planning_lineage = lower_planning_lineage(&source.planning_lineage);
        let config_ref_digest = config_ref_digest(&config_ref)?;
        self.insert_value_lineage(spec::ValueLineage {
            lineage_ref: lineage_ref(verify.output_value_lineage.digest()),
            scope_id: source.scope_id.clone(),
            producer: spec::CellProducer::Node(verify.node_id.clone()),
            input_cells: vec![submit.output_cell.clone()],
            config_ref_digest: Some(config_ref_digest),
            planning_lineage: planning_lineage.clone(),
            domain_keys: verify
                .output_domain_keys
                .iter()
                .map(lower_domain_key_ref)
                .collect(),
            transform_policy: spec::LineageTransformPolicy::StateOutput,
        })?;
        self.insert_cell(spec::CellSpec {
            cell_id: verify.output_cell_id.clone(),
            producer: spec::CellProducer::Node(verify.node_id.clone()),
            scope_id: source.scope_id.clone(),
            semantic_type_id: source.output_semantic_type_id.clone(),
            schema_id: source.output_schema_id.clone(),
            value_lineage: lineage_ref(verify.output_value_lineage.digest()),
            terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
            storage_policy: spec::StoragePolicy::ContentAddressed,
            redaction_policy: spec::RedactionPolicy::Public,
            context: verify.output_context.clone(),
        })?;
        Ok(spec::NodeSpec {
            node_id: verify.node_id.clone(),
            stable_key: spec::side_effect_verify_stable_key(&stable_author_key(
                source.key.as_str(),
            )?)
            .map_err(|error| CertifyError::Spec(error.to_string()))?,
            scope_id: source.scope_id.clone(),
            state_kind: descriptor.state_kind,
            state_version: descriptor.state_version,
            descriptor_id: descriptor.descriptor_id,
            context: node_context,
            config_ref,
            input_bindings: input_binding,
            output_cell: verify.output_cell_id.clone(),
            effect_kind: descriptor.effect_kind,
            capability_bindings: descriptor.capabilities,
            adapter_bindings: Vec::new(),
            fact_descriptor_allowlist: Vec::new(),
            side_effect: None,
            framework: Some(spec::FrameworkNodeSpec::SideEffectVerify(
                spec::SideEffectVerifyNodeSpec {
                    pair_id: verify.pair_id.clone(),
                    submit_node_id: submit.node_id.clone(),
                },
            )),
            planning_lineage,
            deterministic_predecessors: vec![submit.node_id.clone()],
        })
    }

    fn lower_bridge_nodes(&mut self) -> Result<()> {
        for bridge in self.draft.bridge_nodes() {
            let source = self
                .cell_info
                .get(bridge.source_cell_id.as_str())
                .cloned()
                .ok_or_else(|| {
                    problem(
                        ProblemClass::InvalidTopology,
                        format!(
                            "bridge source cell {} has no producer",
                            bridge.source_cell_id
                        ),
                    )
                })?;
            let config_ref = framework_config_ref("bridge_same_value", &bridge.node_id)?;
            self.insert_config_ref(config_ref.clone())?;
            let input_binding =
                single_cell_input_binding(source.clone(), bridge.source_cell_id.clone(), "source")?;
            let planning_lineage = lower_planning_lineage(&bridge.planning_lineage);
            let lineage = spec::ValueLineage {
                lineage_ref: lineage_ref(bridge.target_value_lineage.digest()),
                scope_id: bridge.target_scope_id.clone(),
                producer: spec::CellProducer::Node(bridge.node_id.clone()),
                input_cells: vec![bridge.source_cell_id.clone()],
                config_ref_digest: None,
                planning_lineage: planning_lineage.clone(),
                domain_keys: Vec::new(),
                transform_policy: spec::LineageTransformPolicy::SameValueBridge,
            };
            self.insert_value_lineage(lineage)?;
            self.insert_cell(spec::CellSpec {
                cell_id: bridge.target_cell_id.clone(),
                producer: spec::CellProducer::Node(bridge.node_id.clone()),
                scope_id: bridge.target_scope_id.clone(),
                semantic_type_id: bridge.semantic_type_id.clone(),
                schema_id: bridge.schema_id.clone(),
                value_lineage: lineage_ref(bridge.target_value_lineage.digest()),
                terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
                storage_policy: spec::StoragePolicy::ContentAddressed,
                redaction_policy: spec::RedactionPolicy::Public,
                context: bridge.context.clone(),
            })?;
            let descriptor = framework_bridge_descriptor(
                &bridge.schema_id,
                &bridge.semantic_type_id,
                &config_ref.schema_id,
                &input_binding.input_schema_id,
            )?;
            self.insert_descriptor(spec::DescriptorIdentity::State(Box::new(
                descriptor.clone(),
            )))?;
            self.nodes.push(spec::NodeSpec {
                node_id: bridge.node_id.clone(),
                stable_key: stable_author_key(bridge.key.as_str())?,
                scope_id: bridge.target_scope_id.clone(),
                state_kind: descriptor.state_kind,
                state_version: descriptor.state_version,
                descriptor_id: descriptor.descriptor_id,
                context: spec::NodeContextSpec::no_context(),
                config_ref,
                input_bindings: input_binding,
                output_cell: bridge.target_cell_id.clone(),
                effect_kind: descriptor.effect_kind,
                capability_bindings: descriptor.capabilities,
                adapter_bindings: Vec::new(),
                fact_descriptor_allowlist: Vec::new(),
                side_effect: None,
                framework: Some(spec::FrameworkNodeSpec::Bridge(spec::BridgeNodeSpec {
                    bridge_kind: lower_bridge_kind(bridge.bridge_kind),
                    source_scope_id: bridge.source_scope_id.clone(),
                    target_scope_id: bridge.target_scope_id.clone(),
                    source_cell_id: bridge.source_cell_id.clone(),
                    target_cell_id: bridge.target_cell_id.clone(),
                    semantic_type_id: bridge.semantic_type_id.clone(),
                    schema_id: bridge.schema_id.clone(),
                    policy: lower_bridge_policy(bridge.policy),
                    provenance: spec::BridgeProvenance::FrameworkChildScopeV1,
                })),
                planning_lineage,
                deterministic_predecessors: self
                    .predecessors_for_inputs(std::slice::from_ref(&bridge.source_cell_id))?,
            });
        }
        Ok(())
    }

    fn lower_public_outputs(&mut self) -> Result<spec::PublicOutputSpec> {
        let mut outputs = Vec::new();
        for output in self.draft.public_output_spec().outputs() {
            let cell = output.cell();
            let info = self
                .cell_info
                .get(cell.cell_id().as_str())
                .cloned()
                .ok_or_else(|| {
                    problem(
                        ProblemClass::InvalidTerminalShape,
                        format!("public output cell {} has no producer", cell.cell_id()),
                    )
                })?;
            outputs.push(spec::PublicOutputCell {
                public_field_path: spec::PublicFieldPath::new(output.public_field_path().as_str())
                    .map_err(|error| lower(error.to_string()))?,
                cell_id: cell.cell_id().clone(),
                producer: info.producer,
                scope_id: cell.scope_id().clone(),
                semantic_type_id: cell.semantic_type_id().clone(),
                schema_id: cell.schema_id().clone(),
                value_lineage: lineage_ref(cell.value_lineage().digest()),
                required_terminal: spec::RequiredTerminal::ProducedOnly,
            });
        }
        let renderer_descriptor =
            renderer_descriptor(self.draft.public_output_spec().public_schema_id())?;
        self.insert_descriptor(spec::DescriptorIdentity::Renderer(Box::new(
            renderer_descriptor.clone(),
        )))?;
        let public_outputs = spec::PublicOutputSpec {
            public_schema_id: self.draft.public_output_spec().public_schema_id().clone(),
            outputs,
            renderer_descriptor,
        };
        let render_receipt_cell = self.lower_public_output_render_node(&public_outputs)?;
        let retention_receipt_cell =
            self.lower_project_retention_manifest_node(&public_outputs, render_receipt_cell)?;
        self.lower_complete_run_node(&public_outputs, retention_receipt_cell.clone())?;
        self.lower_resolve_saga_terminal_node(&public_outputs)?;
        Ok(public_outputs)
    }

    fn lower_public_output_render_node(
        &mut self,
        public_outputs: &spec::PublicOutputSpec,
    ) -> Result<CellId> {
        let output_spec_digest = public_outputs
            .digest()
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let node_id = render_node_id(
            self.draft.root_scope_id(),
            self.draft.public_output_spec().key().as_str(),
            &output_spec_digest,
        )?;
        let semantic_type_id = spec::public_output_receipt_semantic_type_id()
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let receipt_schema_id = spec::public_output_receipt_schema_id()
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let output_cell = framework_cell_id(
            self.draft.root_scope_id(),
            &node_id,
            &semantic_type_id,
            &receipt_schema_id,
        )?;
        let planning_lineage = final_planning_lineage(self.draft.operation_lineage())?;
        let config_ref = framework_config_ref("public_output_render", &node_id)?;
        let config_ref_digest = config_ref_digest(&config_ref)?;
        let required_cells = public_outputs.outputs.clone();
        let lineage_ref = render_value_lineage_ref(
            self.draft.root_scope_id(),
            &node_id,
            &required_cells
                .iter()
                .map(|output| output.cell_id.clone())
                .collect::<Vec<_>>(),
            &planning_lineage,
            &config_ref_digest,
        )?;
        self.insert_config_ref(config_ref.clone())?;
        let input_root = spec::InputBindingNodeSpec::Struct(
            required_cells
                .iter()
                .map(|output| {
                    let cell = self
                        .cells
                        .iter()
                        .find(|cell| cell.cell_id == output.cell_id)
                        .ok_or_else(|| {
                            problem(
                                ProblemClass::InvalidTopology,
                                format!("public output cell {} is missing", output.cell_id),
                            )
                        })?;
                    Ok(spec::NamedInputBindingSpec {
                        field_path: output.public_field_path.clone(),
                        node: spec::InputBindingNodeSpec::Cell(Box::new(
                            spec::InputBindingCellSpec {
                                field_path: output.public_field_path.clone(),
                                cell_id: output.cell_id.clone(),
                                semantic_type_id: output.semantic_type_id.clone(),
                                schema_id: output.schema_id.clone(),
                                required_terminal: output.required_terminal,
                                value_lineage: output.value_lineage.clone(),
                                context: input_context_from_cell_context(&cell.context),
                            },
                        )),
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        );
        let input_binding = spec::InputBindingSpec {
            input_schema_id: public_outputs.public_schema_id.clone(),
            input_descriptor_id: descriptor_id_json(serde_json::json!({
                "framework": "public_output_render_input",
                "public_schema_id": public_outputs.public_schema_id.as_str(),
            }))?,
            digest: content_digest_json(input_node_json(&input_root))?,
            root: input_root,
        };
        let (node_context, descriptor_context) = render_context_contract_for_public_outputs(
            &required_cells,
            &self.cells,
            self.draft.contexts(),
        )?;
        let descriptor = framework_render_descriptor(
            &receipt_schema_id,
            &semantic_type_id,
            &config_ref.schema_id,
            &input_binding.input_schema_id,
            descriptor_context,
        )?;
        self.insert_descriptor(spec::DescriptorIdentity::State(Box::new(
            descriptor.clone(),
        )))?;
        self.insert_value_lineage(spec::ValueLineage {
            lineage_ref: lineage_ref.clone(),
            scope_id: self.draft.root_scope_id().clone(),
            producer: spec::CellProducer::Node(node_id.clone()),
            input_cells: sorted_cell_ids(
                required_cells
                    .iter()
                    .map(|output| output.cell_id.clone())
                    .collect(),
            ),
            config_ref_digest: Some(config_ref_digest),
            planning_lineage: planning_lineage.clone(),
            domain_keys: Vec::new(),
            transform_policy: spec::LineageTransformPolicy::StateOutput,
        })?;
        self.insert_cell(spec::CellSpec {
            cell_id: output_cell.clone(),
            producer: spec::CellProducer::Node(node_id.clone()),
            scope_id: self.draft.root_scope_id().clone(),
            semantic_type_id,
            schema_id: receipt_schema_id,
            value_lineage: lineage_ref,
            terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
            storage_policy: spec::StoragePolicy::PublicOutputArtifact,
            redaction_policy: spec::RedactionPolicy::Public,
            context: spec::CellContextSpec::no_context(),
        })?;
        let input_cell_ids = required_cells
            .iter()
            .map(|output| output.cell_id.clone())
            .collect::<Vec<_>>();
        self.nodes.push(spec::NodeSpec {
            node_id,
            stable_key: stable_author_key(self.draft.public_output_spec().key().as_str())?,
            scope_id: self.draft.root_scope_id().clone(),
            state_kind: descriptor.state_kind,
            state_version: descriptor.state_version,
            descriptor_id: descriptor.descriptor_id,
            context: node_context,
            config_ref,
            input_bindings: input_binding,
            output_cell: output_cell.clone(),
            effect_kind: descriptor.effect_kind,
            capability_bindings: descriptor.capabilities,
            adapter_bindings: Vec::new(),
            fact_descriptor_allowlist: Vec::new(),
            side_effect: None,
            framework: Some(spec::FrameworkNodeSpec::PublicOutputRender(
                spec::PublicOutputRenderNodeSpec {
                    public_schema_id: public_outputs.public_schema_id.clone(),
                    output_spec_digest,
                    renderer_descriptor: public_outputs.renderer_descriptor.clone(),
                    required_cells,
                },
            )),
            planning_lineage,
            deterministic_predecessors: self.predecessors_for_inputs(&input_cell_ids)?,
        });
        Ok(output_cell)
    }

    fn lower_project_retention_manifest_node(
        &mut self,
        public_outputs: &spec::PublicOutputSpec,
        public_output_receipt_cell: CellId,
    ) -> Result<CellId> {
        let stable_key = stable_author_key("framework/project-retention-manifest")?;
        let retention = spec::ProjectRetentionManifestNodeSpec {
            public_schema_id: public_outputs.public_schema_id.clone(),
            public_output_receipt_cell: public_output_receipt_cell.clone(),
        };
        let node_id = project_retention_manifest_node_id_from_spec(
            self.draft.root_scope_id(),
            stable_key.as_str(),
            &retention,
        )?;
        let semantic_type_id = spec::retention_manifest_receipt_semantic_type_id()
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let receipt_schema_id = spec::retention_manifest_receipt_schema_id()
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let planning_lineage = final_planning_lineage(self.draft.operation_lineage())?;
        let config_ref = framework_config_ref("project_retention_manifest", &node_id)?;
        let input_cell = self
            .cells
            .iter()
            .find(|cell| cell.cell_id == public_output_receipt_cell)
            .cloned()
            .ok_or_else(|| {
                problem(
                    ProblemClass::InvalidTopology,
                    format!(
                        "retention lifecycle node input cell {public_output_receipt_cell} is missing"
                    ),
                )
            })?;
        let input_binding = spec::framework_lifecycle_receipt_input_binding(
            "project_retention_manifest",
            "public_output_receipt",
            &input_cell,
        )
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let descriptor = framework_project_retention_manifest_descriptor(
            &receipt_schema_id,
            &semantic_type_id,
            &config_ref.schema_id,
            &input_binding.input_schema_id,
        )?;
        self.lower_framework_lifecycle_node(FrameworkLifecycleNode {
            node_id,
            stable_key,
            config_ref,
            input_binding,
            descriptor,
            input_cells: vec![public_output_receipt_cell],
            framework: spec::FrameworkNodeSpec::ProjectRetentionManifest(retention),
            planning_lineage,
        })
    }

    fn lower_complete_run_node(
        &mut self,
        public_outputs: &spec::PublicOutputSpec,
        retention_manifest_receipt_cell: CellId,
    ) -> Result<CellId> {
        let stable_key = stable_author_key("framework/complete-run")?;
        let complete = spec::CompleteRunNodeSpec {
            public_schema_id: public_outputs.public_schema_id.clone(),
            retention_manifest_receipt_cell: retention_manifest_receipt_cell.clone(),
        };
        let node_id = complete_run_node_id_from_spec(
            self.draft.root_scope_id(),
            stable_key.as_str(),
            &complete,
        )?;
        let semantic_type_id = spec::complete_run_receipt_semantic_type_id()
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let receipt_schema_id = spec::complete_run_receipt_schema_id()
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let planning_lineage = final_planning_lineage(self.draft.operation_lineage())?;
        let config_ref = framework_config_ref("complete_run", &node_id)?;
        let input_cell = self
            .cells
            .iter()
            .find(|cell| cell.cell_id == retention_manifest_receipt_cell)
            .cloned()
            .ok_or_else(|| {
                problem(
                    ProblemClass::InvalidTopology,
                    format!(
                        "complete lifecycle node input cell {retention_manifest_receipt_cell} is missing"
                    ),
                )
            })?;
        let input_binding = spec::framework_lifecycle_receipt_input_binding(
            "complete_run",
            "retention_manifest_receipt",
            &input_cell,
        )
        .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let descriptor = framework_complete_run_descriptor(
            &receipt_schema_id,
            &semantic_type_id,
            &config_ref.schema_id,
            &input_binding.input_schema_id,
        )?;
        self.lower_framework_lifecycle_node(FrameworkLifecycleNode {
            node_id,
            stable_key,
            config_ref,
            input_binding,
            descriptor,
            input_cells: vec![retention_manifest_receipt_cell],
            framework: spec::FrameworkNodeSpec::CompleteRun(complete),
            planning_lineage,
        })
    }

    fn lower_resolve_saga_terminal_node(
        &mut self,
        public_outputs: &spec::PublicOutputSpec,
    ) -> Result<CellId> {
        let stable_key = stable_author_key("framework/resolve-saga-terminal")?;
        let resolve = spec::ResolveSagaTerminalNodeSpec {
            public_schema_id: public_outputs.public_schema_id.clone(),
        };
        let node_id = resolve_saga_terminal_node_id_from_spec(
            self.draft.root_scope_id(),
            stable_key.as_str(),
            &resolve,
        )?;
        let semantic_type_id = spec::resolve_saga_terminal_receipt_semantic_type_id()
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let receipt_schema_id = spec::resolve_saga_terminal_receipt_schema_id()
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let planning_lineage = final_planning_lineage(self.draft.operation_lineage())?;
        let config_ref = framework_config_ref("resolve_saga_terminal", &node_id)?;
        let input_binding = spec::framework_lifecycle_unit_input_binding("resolve_saga_terminal")
            .map_err(|error| CertifyError::Spec(error.to_string()))?;
        let descriptor = framework_resolve_saga_terminal_descriptor(
            &receipt_schema_id,
            &semantic_type_id,
            &config_ref.schema_id,
            &input_binding.input_schema_id,
        )?;
        self.lower_framework_lifecycle_node(FrameworkLifecycleNode {
            node_id,
            stable_key,
            config_ref,
            input_binding,
            descriptor,
            input_cells: Vec::new(),
            framework: spec::FrameworkNodeSpec::ResolveSagaTerminal(resolve),
            planning_lineage,
        })
    }

    fn lower_framework_lifecycle_node(
        &mut self,
        lifecycle: FrameworkLifecycleNode,
    ) -> Result<CellId> {
        let FrameworkLifecycleNode {
            node_id,
            stable_key,
            config_ref,
            input_binding,
            descriptor,
            input_cells,
            framework,
            planning_lineage,
        } = lifecycle;
        let scope_id = self.draft.root_scope_id().clone();
        let output_cell = framework_cell_id(
            &scope_id,
            &node_id,
            &descriptor.output_semantic_type_id,
            &descriptor.output_schema_id,
        )?;
        let config_ref_digest = config_ref_digest(&config_ref)?;
        let lineage_ref = render_value_lineage_ref(
            &scope_id,
            &node_id,
            &input_cells,
            &planning_lineage,
            &config_ref_digest,
        )?;
        self.insert_config_ref(config_ref.clone())?;
        self.insert_descriptor(spec::DescriptorIdentity::State(Box::new(
            descriptor.clone(),
        )))?;
        self.insert_value_lineage(spec::ValueLineage {
            lineage_ref: lineage_ref.clone(),
            scope_id: scope_id.clone(),
            producer: spec::CellProducer::Node(node_id.clone()),
            input_cells: input_cells.clone(),
            config_ref_digest: Some(config_ref_digest),
            planning_lineage: planning_lineage.clone(),
            domain_keys: Vec::new(),
            transform_policy: spec::LineageTransformPolicy::StateOutput,
        })?;
        self.insert_cell(spec::CellSpec {
            cell_id: output_cell.clone(),
            producer: spec::CellProducer::Node(node_id.clone()),
            scope_id: scope_id.clone(),
            semantic_type_id: descriptor.output_semantic_type_id.clone(),
            schema_id: descriptor.output_schema_id.clone(),
            value_lineage: lineage_ref,
            terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
            storage_policy: spec::StoragePolicy::ContentAddressed,
            redaction_policy: spec::RedactionPolicy::Public,
            context: spec::CellContextSpec::no_context(),
        })?;
        self.nodes.push(spec::NodeSpec {
            node_id,
            stable_key,
            scope_id,
            state_kind: descriptor.state_kind,
            state_version: descriptor.state_version,
            descriptor_id: descriptor.descriptor_id,
            context: spec::NodeContextSpec::no_context(),
            config_ref,
            input_bindings: input_binding,
            output_cell: output_cell.clone(),
            effect_kind: descriptor.effect_kind,
            capability_bindings: descriptor.capabilities,
            adapter_bindings: Vec::new(),
            fact_descriptor_allowlist: Vec::new(),
            side_effect: None,
            framework: Some(framework),
            planning_lineage,
            deterministic_predecessors: self.predecessors_for_inputs(&input_cells)?,
        });
        Ok(output_cell)
    }

    fn lower_operation_lineage(&mut self) -> Result<Vec<spec::OperationLineageFrameSpec>> {
        let mut frames = Vec::new();
        for frame in self.draft.operation_lineage() {
            self.config_ref(&frame.config)?;
            let input = lower_operation_input_binding(&frame.input)?;
            self.insert_descriptor(spec::DescriptorIdentity::Operation(Box::new(
                operation_descriptor_identity_from_program(frame),
            )))?;
            for output in &frame.output_handles {
                if !self.cell_info.contains_key(output.cell_id().as_str()) {
                    return Err(problem(
                        ProblemClass::InvalidTopology,
                        format!(
                            "operation frame {} returned unknown cell {}",
                            frame.operation_instance_id,
                            output.cell_id()
                        ),
                    ));
                }
            }
            frames.push(spec::OperationLineageFrameSpec {
                operation_instance_id: frame.operation_instance_id.clone(),
                operation_key: stable_author_key(frame.key.as_str())?,
                scope_id: frame.scope_id.clone(),
                operation_descriptor_id: frame.operation_descriptor_id.clone(),
                config_ref_digest: frame.config.config_ref_digest.clone(),
                input_bindings: input.clone(),
                input_binding_digest: input.digest,
                parent_planning_lineage: lower_planning_lineage(&frame.parent_operation_lineage),
                output_cells: frame
                    .output_handles
                    .iter()
                    .map(|handle| handle.cell_id().clone())
                    .collect(),
                lineage_digest: frame.lineage_digest.clone(),
            });
        }
        Ok(frames)
    }

    fn config_ref(&mut self, binding: &program::ConfigBindingSpec) -> Result<spec::ConfigRef> {
        let config_ref = spec::ConfigRef {
            schema_id: binding.schema_id.clone(),
            artifact_id: ArtifactId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                *binding.content_digest.digest(),
            ),
            digest: binding.content_digest.clone(),
            byte_len: binding.byte_len as u64,
            media_type: spec::MediaType::new("application/json")
                .map_err(|error| lower(error.to_string()))?,
        };
        self.insert_config_ref(config_ref.clone())?;
        Ok(config_ref)
    }

    fn insert_config_ref(&mut self, config_ref: spec::ConfigRef) -> Result<()> {
        let key = config_ref_key(&config_ref);
        if let Some(existing) = self.config_refs.get(&key) {
            if existing != &config_ref {
                return Err(problem(
                    ProblemClass::InvalidDataShape,
                    format!(
                        "conflicting config ref for schema {} digest {}",
                        config_ref.schema_id, config_ref.digest
                    ),
                ));
            }
        } else {
            self.config_refs.insert(key, config_ref);
        }
        Ok(())
    }

    fn insert_descriptor(&mut self, descriptor: spec::DescriptorIdentity) -> Result<()> {
        let key = descriptor.descriptor_id().as_str().to_owned();
        if let Some(existing) = self.descriptor_identities.get(&key) {
            if existing != &descriptor {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!("conflicting descriptor identity for {key}"),
                ));
            }
        } else {
            self.descriptor_identities.insert(key, descriptor);
        }
        Ok(())
    }

    fn insert_value_lineage(&mut self, lineage: spec::ValueLineage) -> Result<()> {
        let key = lineage.lineage_ref.lineage_digest.as_str().to_owned();
        if let Some(existing) = self.value_lineages.get(&key) {
            if existing != &lineage {
                return Err(problem(
                    ProblemClass::InvalidDataMeaning,
                    format!("conflicting value lineage for {key}"),
                ));
            }
        } else {
            self.value_lineages.insert(key, lineage);
        }
        Ok(())
    }

    fn insert_cell(&mut self, cell: spec::CellSpec) -> Result<()> {
        self.register_planned_cell_info(
            CellInfo {
                producer: cell.producer.clone(),
                semantic_type_id: cell.semantic_type_id.clone(),
                schema_id: cell.schema_id.clone(),
                value_lineage: cell.value_lineage.clone(),
                context: cell.context.clone(),
            },
            &cell.cell_id,
        )?;
        if self
            .cells
            .iter()
            .any(|existing| existing.cell_id == cell.cell_id)
        {
            return Err(problem(
                ProblemClass::InvalidTopology,
                format!("duplicate cell id {}", cell.cell_id),
            ));
        }
        self.cells.push(cell);
        Ok(())
    }

    fn register_planned_cell_info(&mut self, info: CellInfo, cell_id: &CellId) -> Result<()> {
        let key = cell_id.as_str().to_owned();
        if let Some(existing) = self.cell_info.get(&key) {
            if existing != &info {
                return Err(problem(
                    ProblemClass::InvalidTopology,
                    format!("conflicting planned cell metadata for {cell_id}"),
                ));
            }
        } else {
            self.cell_info.insert(key, info);
        }
        Ok(())
    }

    fn predecessors_for_inputs(&self, input_cells: &[CellId]) -> Result<Vec<NodeId>> {
        let mut predecessors = BTreeSet::new();
        for cell_id in input_cells {
            match self
                .cell_info
                .get(cell_id.as_str())
                .ok_or_else(|| {
                    problem(
                        ProblemClass::InvalidTopology,
                        format!("input cell {cell_id} has no producer"),
                    )
                })?
                .producer
                .clone()
            {
                spec::CellProducer::Node(node_id) => {
                    predecessors.insert(node_id);
                }
                spec::CellProducer::Seed(_) => {}
            }
        }
        Ok(predecessors.into_iter().collect())
    }

    fn authoring_provenance(&self) -> Result<spec::AuthoringProvenance> {
        let frame_digests = self
            .draft
            .operation_lineage()
            .iter()
            .map(|frame| frame.lineage_digest.as_str())
            .collect::<Vec<_>>();
        let descriptor = spec::CompositionDescriptor {
            descriptor_id: descriptor_id_json(serde_json::json!({
                "kind": "typed_program_draft",
                "root_scope_id": self.draft.root_scope_id().as_str(),
                "operation_frames": frame_digests,
                "state_nodes": self.draft.state_nodes().len(),
            }))?,
            name: "typed-program-draft".to_owned(),
            version: "mfm.typed.program_draft.v1".to_owned(),
        };
        let config_hash = content_digest_json(serde_json::json!({
            "root_key": self.draft.root_key().as_str(),
            "root_scope_id": self.draft.root_scope_id().as_str(),
        }))?;
        if self.draft.operation_lineage().is_empty() {
            Ok(spec::AuthoringProvenance::StateComposition {
                descriptor,
                config_hash,
            })
        } else {
            Ok(spec::AuthoringProvenance::MixedComposition {
                descriptor,
                config_hash,
            })
        }
    }
}
