use super::*;

impl ReplayBroker {
    pub(super) fn verify_public_output_against_spec(
        &self,
        payload: &events::PublicOutputProduced,
    ) -> Result<()> {
        let render = self.verify_public_output_header(
            payload.node_id.clone(),
            &payload.receipt_cell_id,
            &payload.public_schema_id,
            &payload.renderer_descriptor_id,
        )?;
        if payload.output_spec_digest != render.output_spec_digest
            || payload.cells.len() != render.required_cells.len()
        {
            return Err(certified_evidence_mismatch(
                "public output event does not match certified render contract",
            ));
        }
        for (actual, expected) in payload.cells.iter().zip(render.required_cells.iter()) {
            if actual.public_field_path != expected.public_field_path
                || actual.cell_id != expected.cell_id
                || actual.producer != expected.producer
                || actual.scope_id != expected.scope_id
                || actual.semantic_type_id != expected.semantic_type_id
                || actual.schema_id != expected.schema_id
                || actual.value_lineage != expected.value_lineage
            {
                return Err(certified_evidence_mismatch(
                    "public output cell does not match certified spec",
                ));
            }
            self.verify_public_output_cell_projection(actual)?;
        }
        Ok(())
    }

    pub(super) fn verify_public_output_render_failure_against_spec(
        &self,
        payload: &events::PublicOutputRenderFailed,
    ) -> Result<()> {
        self.verify_public_output_header(
            payload.node_id.clone(),
            &self.node(&payload.node_id)?.output_cell.clone(),
            &payload.public_schema_id,
            &payload.renderer_descriptor_id,
        )?;
        Ok(())
    }

    fn verify_public_output_header(
        &self,
        node_id: NodeId,
        receipt_cell_id: &mfm_ids::CellId,
        public_schema_id: &SchemaId,
        renderer_descriptor_id: &mfm_ids::DescriptorId,
    ) -> Result<&spec::PublicOutputRenderNodeSpec> {
        let node = self.node(&node_id)?;
        let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &node.framework else {
            return Err(certified_evidence_mismatch(
                "public output event node is not a certified public-output render node",
            ));
        };
        if node.output_cell != *receipt_cell_id
            || render.public_schema_id != *public_schema_id
            || self.certified_spec.spec.public_outputs.public_schema_id != *public_schema_id
            || render.renderer_descriptor.descriptor_id != *renderer_descriptor_id
        {
            return Err(certified_evidence_mismatch(
                "public output event does not match certified public-output contract",
            ));
        }
        Ok(render)
    }

    fn verify_public_output_cell_projection(&self, cell: &events::NamedTypedCellRef) -> Result<()> {
        let Some(projection) = self
            .projection
            .cell_terminal_for_run(&self.run_id.run_id, &cell.cell_id)
        else {
            return Err(certified_evidence_mismatch(
                "public output cell has no terminal projection",
            ));
        };
        match projection {
            store::CellTerminalProjection::Produced {
                schema_id,
                semantic_type_id,
                artifact_id,
                content_digest,
                evidence_hash,
                ..
            } if schema_id == &cell.schema_id
                && semantic_type_id == &cell.semantic_type_id
                && artifact_id == &cell.artifact_id
                && content_digest == &cell.content_digest
                && evidence_hash == &cell.evidence_hash =>
            {
                Ok(())
            }
            _ => Err(certified_evidence_mismatch(
                "public output cell evidence does not match produced cell projection",
            )),
        }
    }

    pub(super) fn verify_completed_run_public_output(
        &self,
        evidence: &events::PublicOutputCompletionEvidence,
    ) -> Result<()> {
        if evidence.public_output_schema_id
            != self.certified_spec.spec.public_outputs.public_schema_id
        {
            return Err(certified_evidence_mismatch(
                "run completion public-output schema does not match certified spec",
            ));
        }
        match self
            .projection
            .public_output(&self.run_id.run_id, &evidence.public_output_schema_id)
        {
            Some(store::PublicOutputProjection::Produced { event_id, .. })
                if event_id == &evidence.public_output_event_id =>
            {
                Ok(())
            }
            _ => Err(certified_evidence_mismatch(
                "run completion public-output evidence does not match projected output",
            )),
        }
    }
}
