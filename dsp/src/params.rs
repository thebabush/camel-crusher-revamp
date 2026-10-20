//! Parameter table, in CamelCrusher's parameter order. Values are normalized 0..1.

pub const NUM_PARAMS: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum ParamId {
    DistOn = 0,
    DistMech,
    DistTube,
    MmFilterOn,
    MmFilterCutoff,
    MmFilterRes,
    CompressOn,
    CompressAmount,
    /// The "P" (Phat) button.
    CompressMode,
    MasterOn,
    MasterMix,
    MasterVolume,
}

impl ParamId {
    pub const ALL: [ParamId; NUM_PARAMS] = [
        ParamId::DistOn,
        ParamId::DistMech,
        ParamId::DistTube,
        ParamId::MmFilterOn,
        ParamId::MmFilterCutoff,
        ParamId::MmFilterRes,
        ParamId::CompressOn,
        ParamId::CompressAmount,
        ParamId::CompressMode,
        ParamId::MasterOn,
        ParamId::MasterMix,
        ParamId::MasterVolume,
    ];
}

/// Cutoff knob to Hz, for display.
pub fn cutoff_hz(v: f32) -> f32 {
    crate::filter::cutoff_to_hz(v)
}

/// Compress Amount is displayed as `20*log10(1 - 0.99 v)` dB, i.e. the negative
/// of the makeup gain driven into the fixed knee (0 to -40 dB).
pub fn compress_amount_db(v: f32) -> f32 {
    20.0 * (1.0 - 0.99 * v).log10()
}

/// Master volume knob to linear gain.
pub fn volume_gain(v: f32) -> f32 {
    v.powf(2.737)
}

/// Inverse of [`cutoff_hz`], clamped to the knob range.
pub fn cutoff_from_hz(hz: f32) -> f32 {
    let t = ((hz - 80.0) / 15920.0).clamp(0.0, 1.0);
    1.0 - 10f32.powf(-4.0 * t)
}

/// Inverse of [`compress_amount_db`], clamped to the knob range.
pub fn compress_amount_from_db(db: f32) -> f32 {
    ((1.0 - 10f32.powf(db / 20.0)) / 0.99).clamp(0.0, 1.0)
}

/// Inverse of [`volume_gain`], clamped to the knob range.
pub fn volume_from_gain(gain: f32) -> f32 {
    gain.clamp(0.0, 1.0).powf(1.0 / 2.737)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverses_roundtrip() {
        for i in 0..=1000 {
            let v = i as f32 / 1000.0;
            if v < 0.9999 {
                assert!(
                    (cutoff_from_hz(cutoff_hz(v)) - v).abs() < 1e-3,
                    "cutoff {v}"
                );
            }
            assert!((compress_amount_from_db(compress_amount_db(v)) - v).abs() < 1e-4);
            assert!((volume_from_gain(volume_gain(v)) - v).abs() < 1e-4);
        }
    }
}
