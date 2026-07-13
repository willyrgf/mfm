use super::*;

/// Child-scope builder with live bridge-session authority.
pub struct ChildScopeBuilder<'program, 'parent, 'child> {
    pub(super) parent_scope_id: ScopeId,
    pub(super) scope: ScopeBuilder<'program, 'child>,
    pub(super) session_token: BridgeSessionToken,
    pub(super) bridge_keys: BTreeSet<String>,
    pub(super) active_bridge_refs: BTreeSet<String>,
    pub(super) bridge_nodes: Vec<BridgeNodeSpec>,
    pub(super) _parent: PhantomData<fn(&'parent ()) -> &'parent ()>,
}

impl<'program, 'parent, 'child> ChildScopeBuilder<'program, 'parent, 'child> {
    /// Returns the child scope builder for nested child scopes.
    pub fn scope(&mut self) -> &mut ScopeBuilder<'program, 'child> {
        &mut self.scope
    }

    /// Returns the child scope id.
    pub fn scope_id(&self) -> &ScopeId {
        self.scope.scope_id()
    }

    /// Imports a parent handle into this child scope through an emitted bridge node.
    pub fn import_from_parent<T: MfmValue>(
        &mut self,
        key: BridgeKey,
        value: Handle<'program, 'parent, T>,
        policy: BridgePolicy,
    ) -> Result<Handle<'program, 'child, T>> {
        let source = value.typed_ref();
        let (target, evidence) = self.emit_bridge(
            key,
            BridgeKind::ImportFromParent,
            source,
            self.parent_scope_id.clone(),
            self.scope.scope_id().clone(),
            policy,
        )?;
        Ok(Handle::new_bridge(
            target.cell_id,
            target.scope_id,
            target.schema_id,
            target.semantic_type_id,
            target.value_lineage,
            target.context,
            evidence,
        ))
    }

    /// Exports a child handle into the parent scope through an emitted bridge node.
    pub fn export_to_parent<T: MfmValue>(
        &mut self,
        key: BridgeKey,
        value: Handle<'program, 'child, T>,
        policy: BridgePolicy,
    ) -> Result<Handle<'program, 'parent, T>> {
        let source = value.typed_ref();
        let (target, evidence) = self.emit_bridge(
            key,
            BridgeKind::ExportToParent,
            source,
            self.scope.scope_id().clone(),
            self.parent_scope_id.clone(),
            policy,
        )?;
        Ok(Handle::new_bridge(
            target.cell_id,
            target.scope_id,
            target.schema_id,
            target.semantic_type_id,
            target.value_lineage,
            target.context,
            evidence,
        ))
    }

    /// Wraps parent-visible values after validating their live bridge evidence.
    pub fn bridge_to_parent<R>(&mut self, value: R) -> Result<Bridged<'program, 'parent, R>>
    where
        R: BridgeableToParent<'program, 'parent>,
    {
        let evidence = value.bridge_evidence();
        self.validate_bridge_evidence_set(&evidence)?;
        Ok(Bridged {
            value,
            bridge_evidence: evidence,
            _program: PhantomData,
            _parent: PhantomData,
            _private: (),
        })
    }

    fn emit_bridge(
        &mut self,
        key: BridgeKey,
        bridge_kind: BridgeKind,
        source: TypedHandleRef,
        source_scope_id: ScopeId,
        target_scope_id: ScopeId,
        policy: BridgePolicy,
    ) -> Result<(TypedHandleRef, BridgeEvidenceCore)> {
        if source.scope_id != source_scope_id {
            return Err(PlanError::InvalidBridgeEvidence(format!(
                "source handle scope {} did not match bridge source {}",
                source.scope_id.as_str(),
                source_scope_id.as_str()
            )));
        }
        if !self.bridge_keys.insert(key.as_str().to_owned()) {
            return Err(PlanError::DuplicateBridgeKey(key.as_str().to_owned()));
        }

        let node_id = bridge_node_id(
            &source_scope_id,
            &target_scope_id,
            &source.cell_id,
            &key,
            bridge_kind,
            policy,
        )?;
        let target_cell_id = bridge_cell_id(
            &target_scope_id,
            &node_id,
            &source.semantic_type_id,
            &source.schema_id,
        )?;
        let planning_lineage = self.scope.current_operation_lineage()?;
        let target_value_lineage = value_lineage_ref(&bridge_value_lineage(
            &target_scope_id,
            &node_id,
            &source.cell_id,
            &planning_lineage,
        )?)?;
        let spec = BridgeNodeSpec {
            node_id: node_id.clone(),
            key,
            source_scope_id,
            target_scope_id: target_scope_id.clone(),
            source_cell_id: source.cell_id.clone(),
            target_cell_id: target_cell_id.clone(),
            target_value_lineage: target_value_lineage.clone(),
            semantic_type_id: source.semantic_type_id.clone(),
            schema_id: source.schema_id.clone(),
            context: source.context.clone(),
            bridge_kind,
            policy,
            provenance: BridgeProvenance::FrameworkChildScopeV1,
            planning_lineage,
        };
        let bridge_ref = spec.bridge_ref();
        self.active_bridge_refs.insert(bridge_ref_key(&bridge_ref));
        self.bridge_nodes.push(spec);
        let target = TypedHandleRef {
            cell_id: target_cell_id,
            scope_id: target_scope_id,
            schema_id: source.schema_id,
            semantic_type_id: source.semantic_type_id,
            value_lineage: target_value_lineage,
            context: source.context,
        };
        let evidence = BridgeEvidenceCore {
            bridge_ref,
            session_token: self.session_token,
        };
        Ok((target, evidence))
    }

    pub(super) fn validate_bridge_evidence_set(
        &self,
        evidence_set: &[BridgeEvidence<'program, 'parent>],
    ) -> Result<()> {
        for evidence in evidence_set {
            self.validate_bridge_evidence(evidence)?;
        }
        Ok(())
    }

    fn validate_bridge_evidence(&self, evidence: &BridgeEvidence<'program, 'parent>) -> Result<()> {
        let bridge_ref = &evidence.core.bridge_ref;
        if evidence.core.session_token != self.session_token {
            if bridge_ref.target_scope_id == self.parent_scope_id
                && bridge_ref.source_scope_id != *self.scope.scope_id()
            {
                return Ok(());
            }
            return Err(PlanError::InvalidBridgeEvidence(
                "bridge evidence belongs to a different child-scope session".to_owned(),
            ));
        }
        if bridge_ref.source_scope_id != *self.scope.scope_id()
            || bridge_ref.target_scope_id != self.parent_scope_id
        {
            return Err(PlanError::InvalidBridgeEvidence(
                "bridge evidence does not export from this child to its parent".to_owned(),
            ));
        }
        if !self
            .active_bridge_refs
            .contains(&bridge_ref_key(bridge_ref))
        {
            return Err(PlanError::InvalidBridgeEvidence(
                "bridge ref was not created by this child-scope builder".to_owned(),
            ));
        }
        Ok(())
    }
}
