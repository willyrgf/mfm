use crate::{ProgramError, ProgramLimits, Result};
use std::num::NonZeroU64;

/// Positive maximum complete conclusion-frame bytes for one Pure or Read occurrence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct ConclusionBound(NonZeroU64);

impl ConclusionBound {
    /// Checks positivity; Runtime admission additionally checks Journal's format ceiling.
    pub fn new(max_frame_bytes: u64) -> Result<Self> {
        NonZeroU64::new(max_frame_bytes)
            .map(Self)
            .ok_or(ProgramError::InvalidContract)
    }

    /// Maximum bytes, including envelope and complete frame-local object closure.
    pub const fn max_frame_bytes(self) -> u64 {
        self.0.get()
    }
}

/// Complete prepare and conclusion bounds for an Effect's retained lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "EffectBoundsWire", into = "EffectBoundsWire")]
pub struct EffectBounds {
    prepare: ConclusionBound,
    conclusion: ConclusionBound,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct EffectBoundsWire {
    prepare: u64,
    conclusion: u64,
}
impl From<EffectBounds> for EffectBoundsWire {
    fn from(value: EffectBounds) -> Self {
        Self {
            prepare: value.prepare.max_frame_bytes(),
            conclusion: value.conclusion.max_frame_bytes(),
        }
    }
}
impl TryFrom<EffectBoundsWire> for EffectBounds {
    type Error = ProgramError;
    fn try_from(value: EffectBoundsWire) -> Result<Self> {
        Self::new(value.prepare, value.conclusion)
    }
}

impl EffectBounds {
    /// Checks positive bounds and a representable complete lifecycle sum.
    pub fn new(prepare_bytes: u64, conclusion_bytes: u64) -> Result<Self> {
        let prepare = ConclusionBound::new(prepare_bytes)?;
        let conclusion = ConclusionBound::new(conclusion_bytes)?;
        prepare_bytes
            .checked_add(conclusion_bytes)
            .ok_or(ProgramError::Capacity)?;
        Ok(Self {
            prepare,
            conclusion,
        })
    }

    /// Complete maximum preparation frame bytes.
    pub const fn prepare_bytes(self) -> u64 {
        self.prepare.max_frame_bytes()
    }
    /// Complete maximum settlement frame bytes, including terminal failure alternatives.
    pub const fn conclusion_bytes(self) -> u64 {
        self.conclusion.max_frame_bytes()
    }
}

pub(crate) enum LifecycleBound {
    Conclusion(ConclusionBound),
    Effect(EffectBounds),
}

/// Conservative complete-history cost, derived without a persisted reservation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryBound {
    frames: u64,
    bytes: u64,
}

impl HistoryBound {
    /// Maximum total frames including genesis and all admitted recovery segments.
    pub const fn frames(self) -> u64 {
        self.frames
    }
    /// Maximum total canonical frame bytes including every repeated object closure.
    pub const fn bytes(self) -> u64 {
        self.bytes
    }

    pub(crate) fn calculate(
        genesis: ConclusionBound,
        limits: ProgramLimits,
        sequence: impl IntoIterator<Item = LifecycleBound>,
    ) -> Result<Self> {
        let mut frames = 0_u64;
        let mut bytes = 0_u64;
        for lifecycle in sequence {
            let (count, size) = match lifecycle {
                LifecycleBound::Conclusion(bound) => (1, bound.max_frame_bytes()),
                LifecycleBound::Effect(bounds) => (
                    2,
                    bounds
                        .prepare_bytes()
                        .checked_add(bounds.conclusion_bytes())
                        .ok_or(ProgramError::Capacity)?,
                ),
            };
            frames = frames.checked_add(count).ok_or(ProgramError::Capacity)?;
            bytes = bytes.checked_add(size).ok_or(ProgramError::Capacity)?;
        }
        let segments = u64::from(limits.max_recovery_decisions()) + 1;
        Ok(Self {
            frames: frames
                .checked_mul(segments)
                .and_then(|value| value.checked_add(1))
                .ok_or(ProgramError::Capacity)?,
            bytes: bytes
                .checked_mul(segments)
                .and_then(|value| value.checked_add(genesis.max_frame_bytes()))
                .ok_or(ProgramError::Capacity)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_bound_covers_repeated_segments_and_full_pending_effect_settlement() {
        let bound = HistoryBound::calculate(
            ConclusionBound::new(100).unwrap(),
            ProgramLimits::new(2),
            [
                LifecycleBound::Conclusion(ConclusionBound::new(200).unwrap()),
                LifecycleBound::Effect(EffectBounds::new(300, 400).unwrap()),
                LifecycleBound::Conclusion(ConclusionBound::new(500).unwrap()),
            ],
        )
        .unwrap();
        assert_eq!(bound.frames(), 13);
        assert_eq!(bound.bytes(), 4_300);
        let empty = HistoryBound::calculate(
            ConclusionBound::new(100).unwrap(),
            ProgramLimits::new(u32::MAX),
            [],
        )
        .unwrap();
        assert_eq!(empty.frames(), 1);
        assert_eq!(empty.bytes(), 100);
    }

    #[test]
    fn history_bound_rejects_zero_and_every_arithmetic_overflow() {
        assert_eq!(ConclusionBound::new(0), Err(ProgramError::InvalidContract));
        assert_eq!(EffectBounds::new(0, 1), Err(ProgramError::InvalidContract));
        assert_eq!(EffectBounds::new(1, 0), Err(ProgramError::InvalidContract));
        assert_eq!(EffectBounds::new(u64::MAX, 1), Err(ProgramError::Capacity));
        for (decisions, sizes) in [
            (0, vec![u64::MAX]),
            (1, vec![u64::MAX / 2 + 1]),
            (0, vec![u64::MAX - 1, 2]),
        ] {
            assert_eq!(
                HistoryBound::calculate(
                    ConclusionBound::new(1).unwrap(),
                    ProgramLimits::new(decisions),
                    sizes.into_iter().map(|size| LifecycleBound::Conclusion(
                        ConclusionBound::new(size).unwrap()
                    ))
                ),
                Err(ProgramError::Capacity)
            );
        }
    }
}
