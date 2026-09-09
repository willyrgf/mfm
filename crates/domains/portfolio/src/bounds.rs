//! Concrete Portfolio continuation and complete closure bounds.

use super::*;
use mfm_program::ConclusionBound;

fn bytes<T: mfm_values::MfmValue>(value: &T) -> Result<u64, PortfolioError> {
    mfm_values::canonicalize_mfm_value(value)
        .map(|(canonical, _)| canonical.as_bytes().len() as u64)
        .map_err(|_| PortfolioError::Program)
}
fn add(left: u64, right: u64) -> Result<u64, PortfolioError> {
    left.checked_add(right).ok_or(PortfolioError::Program)
}

pub(super) fn conclusion_bounds(
    input: &PortfolioSnapshotInput,
) -> Result<(ConclusionBound, Vec<ConclusionBound>), PortfolioError> {
    input.validate()?;
    let input_bytes = bytes(input)?;
    let mut completed = 0_u64;
    for collection in &input.collections {
        // Collection header: ordinal, chain, full-width anchor and JSON field names.
        completed = add(completed, 512)?;
        for source in collection.request.sources() {
            // A holding shares the escaped source identity and asset with this source;
            // 512 extra bytes cover raw U256, decimal amount (at most 81 quoted bytes), scale,
            // asset tagging, field names and separators. Addresses/chain are overcounted.
            completed = add(completed, add(bytes(source)?, 512)?)?;
        }
    }
    let continuation = add(add(input_bytes, completed)?, 64)?;
    // Output repeats the portfolio identity, retains all holdings and adds one bounded
    // summary per collection. At most 64 U256 amounts and 30 fractional digits need
    // fewer than 112 characters per total; 256 bytes also cover each summary's keys.
    let summaries = (input.collections.len() as u64)
        .checked_mul(256)
        .ok_or(PortfolioError::Program)?;
    // This also covers enrichment: its paired configs retain a subset of these sources,
    // while collection headers cover route/anchor metadata and the selected quote.
    let output = add(
        add(
            add(
                input_bytes.checked_mul(2).ok_or(PortfolioError::Program)?,
                completed,
            )?,
            summaries,
        )?,
        4096,
    )?;
    // Failure codes admit 256 public bytes, conservatively six JSON bytes each, plus
    // the closed failure tag and ordinal. This also bounds mapped EVM root failures.
    let root_failure = 2048_u64;
    let mut root_frame = add(continuation.max(output).max(root_failure), 16384)?;
    let mut children = Vec::with_capacity(input.collections.len());
    for collection in &input.collections {
        let bound = collection
            .request
            .conclusion_bound(continuation, root_failure)
            .map_err(|_| PortfolioError::Program)?;
        root_frame = root_frame.max(bound.max_frame_bytes());
        children.push(bound);
    }
    Ok((
        ConclusionBound::new(root_frame).map_err(|_| PortfolioError::Program)?,
        children,
    ))
}

#[cfg(test)]
mod tests;
