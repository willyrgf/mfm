//! Conservative balance closure arithmetic for concrete callers.

use super::*;

fn bytes<T: MfmValueTrait>(value: &T) -> Result<u64, EvmDomainError> {
    mfm_values::canonicalize_mfm_value(value)
        .map(|(canonical, _)| canonical.as_bytes().len() as u64)
        .map_err(|_| EvmDomainError::Program)
}
fn add(left: u64, right: u64) -> Result<u64, EvmDomainError> {
    left.checked_add(right).ok_or(EvmDomainError::Program)
}

impl EvmBalanceRequest {
    /// Bounds every collection context/completion using this exact request and a caller-owned bound.
    /// The caller must bound all reachable continuations, including completed earlier collections.
    pub fn maximum_context_bytes(&self, caller_bytes: u64) -> Result<u64, EvmDomainError> {
        self.validate()?;
        let mut completed = 0_u64;
        for source in self.sources() {
            // Each result adds an anchor (78 decimal digits and 66 hash bytes), an 80-byte
            // quoted U256, a two-digit scale, field names and punctuation to its source.
            completed = add(completed, add(bytes(source)?, 512)?)?;
        }
        // Metadata includes a maximally escaped 256-byte correlation, a 512-byte schema
        // identity/reference and ordinal. Work adds one anchor, U256 and scale. This also
        // covers completion wrappers, the maximum 110-digit scaled sum and array separators.
        add(add(add(bytes(self)?, caller_bytes)?, completed)?, 4096)
    }

    /// Bounds complete Read/Pure conclusions, including the caller's mapped root failure.
    /// This is a declared conservative bound; Runtime checks actual objects, frames and run cost.
    pub fn conclusion_bound(
        &self,
        caller_bytes: u64,
        root_failure_bytes: u64,
    ) -> Result<mfm_program::ConclusionBound, EvmDomainError> {
        let context = self.maximum_context_bytes(caller_bytes)?;
        let source = self
            .sources()
            .iter()
            .map(bytes)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .max()
            .ok_or(EvmDomainError::InvalidValue)?;
        // Intent adds route/reference, chain and anchor. Evidence adds an exact intent
        // reference and one anchor/U256/closed code. Failure stages/codes are closed strings.
        let intent = add(source, 2048)?;
        let domain = context.max(add(root_failure_bytes, 512)?);
        let observed = add(1024, domain)?;
        let operational = add(intent, 128)?;
        // Four frame-local objects need at most eight copies of bounded content references,
        // plus the fixed run/head/position/decision wire. 16 KiB conservatively covers this
        // envelope; it is a workload bound, not a duplicated Journal format ceiling.
        let frame = add(add(intent, observed.max(operational))?, 16384)?;
        mfm_program::ConclusionBound::new(frame).map_err(|_| EvmDomainError::Program)
    }
}

#[cfg(test)]
mod tests;
