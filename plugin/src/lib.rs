// vst3-sys's COM macros end an expression with `;`, which newer rustc warns about.
#![cfg_attr(
    feature = "vst3",
    allow(semicolon_in_expressions_from_non_local_macros)
)]

use dune_crusher_dsp::params::{
    compress_amount_db, compress_amount_from_db, cutoff_from_hz, cutoff_hz, volume_from_gain,
    volume_gain,
};
use dune_crusher_dsp::{DuneCrusher, ParamId, FACTORY_PRESETS};
use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use std::sync::Arc;

mod editor;

/// Largest block handed to the DSP at once; host buffers are split to fit.
const MAX_BLOCK: usize = 1024;

pub struct Crusher {
    params: Arc<CrusherParams>,
    dsp: DuneCrusher,
    in_l: Vec<f32>,
    in_r: Vec<f32>,
}

#[derive(Params)]
pub struct CrusherParams {
    #[persist = "editor-state"]
    editor_state: Arc<EguiState>,
    #[id = "dist_on"]
    dist_on: BoolParam,
    #[id = "mech"]
    mech: FloatParam,
    #[id = "tube"]
    tube: FloatParam,
    #[id = "filter_on"]
    filter_on: BoolParam,
    #[id = "cutoff"]
    cutoff: FloatParam,
    #[id = "res"]
    res: FloatParam,
    #[id = "comp_on"]
    comp_on: BoolParam,
    #[id = "comp_amount"]
    comp_amount: FloatParam,
    #[id = "phat"]
    phat: BoolParam,
    #[id = "master_on"]
    master_on: BoolParam,
    #[id = "mix"]
    mix: FloatParam,
    #[id = "volume"]
    volume: FloatParam,
}

fn percent(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
        .with_unit(" %")
}

fn db(v: f32) -> String {
    if v <= -100.0 {
        "-inf".into()
    } else {
        format!("{v:.1}")
    }
}

/// Parses a number with an optional trailing unit, as typed by the user or round-tripped by
/// the host (which may include the unit).
fn parse(s: &str, unit: &str) -> Option<f32> {
    let s = s.trim();
    let s = s.strip_suffix(unit).unwrap_or(s).trim();
    if s.eq_ignore_ascii_case("-inf") {
        Some(f32::NEG_INFINITY)
    } else {
        s.parse().ok()
    }
}

impl CrusherParams {
    /// Defaults are the first factory preset ("Annihilate").
    fn new() -> Self {
        let p = &FACTORY_PRESETS[0].values;
        Self {
            editor_state: EguiState::from_size(460, 500),
            dist_on: BoolParam::new("Distortion On", p[0] >= 0.5),
            mech: percent("Mech", p[1]),
            tube: percent("Tube", p[2]),
            filter_on: BoolParam::new("Filter On", p[3] >= 0.5),
            cutoff: FloatParam::new("Cutoff", p[4], FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_value_to_string(Arc::new(|v| format!("{:.0}", cutoff_hz(v))))
                .with_string_to_value(Arc::new(|s| parse(s, "Hz").map(cutoff_from_hz)))
                .with_unit(" Hz"),
            res: percent("Resonance", p[5]),
            comp_on: BoolParam::new("Compressor On", p[6] >= 0.5),
            comp_amount: FloatParam::new("Amount", p[7], FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_value_to_string(Arc::new(|v| db(compress_amount_db(v))))
                .with_string_to_value(Arc::new(|s| parse(s, "dB").map(compress_amount_from_db)))
                .with_unit(" dB"),
            phat: BoolParam::new("Phat", p[8] >= 0.5),
            master_on: BoolParam::new("Master On", p[9] >= 0.5),
            mix: percent("Mix", p[10]),
            volume: FloatParam::new("Volume", p[11], FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_value_to_string(Arc::new(|v| {
                    let g = volume_gain(v);
                    db(if g < 1e-5 { -200.0 } else { 20.0 * g.log10() })
                }))
                .with_string_to_value(Arc::new(|s| {
                    parse(s, "dB").map(|d| volume_from_gain(10f32.powf(d / 20.0)))
                }))
                .with_unit(" dB"),
        }
    }

    fn values(&self) -> [f32; dune_crusher_dsp::NUM_PARAMS] {
        let b = |p: &BoolParam| if p.value() { 1.0 } else { 0.0 };
        [
            b(&self.dist_on),
            self.mech.value(),
            self.tube.value(),
            b(&self.filter_on),
            self.cutoff.value(),
            self.res.value(),
            b(&self.comp_on),
            self.comp_amount.value(),
            b(&self.phat),
            b(&self.master_on),
            self.mix.value(),
            self.volume.value(),
        ]
    }
}

impl Default for Crusher {
    fn default() -> Self {
        Self {
            params: Arc::new(CrusherParams::new()),
            dsp: DuneCrusher::new(),
            in_l: Vec::new(),
            in_r: Vec::new(),
        }
    }
}

impl Crusher {
    /// The DSP ramps parameters itself (10 ms), so values are pushed
    /// unsmoothed once per block.
    fn push_params(&mut self) {
        for (id, v) in ParamId::ALL.into_iter().zip(self.params.values()) {
            self.dsp.set_param(id, v);
        }
    }
}

impl Plugin for Crusher {
    const NAME: &'static str = "Dune Crusher";
    const VENDOR: &'static str = "dune-crusher";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(self.params.clone())
    }

    fn initialize(
        &mut self,
        _layout: &AudioIOLayout,
        config: &BufferConfig,
        _ctx: &mut impl InitContext<Self>,
    ) -> bool {
        self.dsp = DuneCrusher::new();
        self.dsp.set_sample_rate(config.sample_rate);
        self.in_l = vec![0.0; MAX_BLOCK];
        self.in_r = vec![0.0; MAX_BLOCK];
        self.push_params();
        self.dsp.reset();
        true
    }

    fn reset(&mut self) {
        self.push_params();
        self.dsp.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _ctx: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        self.push_params();
        let total = buffer.samples();
        let [l, r] = buffer.as_slice() else {
            return ProcessStatus::Normal;
        };
        let mut pos = 0;
        while pos < total {
            let n = (total - pos).min(MAX_BLOCK);
            let (il, ir) = (&mut self.in_l[..n], &mut self.in_r[..n]);
            il.copy_from_slice(&l[pos..pos + n]);
            ir.copy_from_slice(&r[pos..pos + n]);
            self.dsp
                .process(il, ir, &mut l[pos..pos + n], &mut r[pos..pos + n]);
            pos += n;
        }
        ProcessStatus::Normal
    }
}

impl ClapPlugin for Crusher {
    const CLAP_ID: &'static str = "com.dune-crusher.dune-crusher";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Dune Crusher: distortion, resonant low-pass and compressor");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::Distortion,
    ];
}

#[cfg(feature = "vst3")]
impl Vst3Plugin for Crusher {
    const VST3_CLASS_ID: [u8; 16] = *b"DuneCrusherAudio";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Distortion];
}

nih_export_clap!(Crusher);
#[cfg(feature = "vst3")]
nih_export_vst3!(Crusher);
