//! Dune Crusher DSP, recreating the original CamelCrusher signal chain (Camel Audio, 2005-2011).
//!
//! Single-precision arithmetic, operation order and the startup quirks (ramps from zero,
//! the filter's initial sweep) are deliberate parts of the sound and are kept as-is.
//!
//! Signal flow per 64-sample chunk:
//!
//! ```text
//! in -> [Mech] -> [Filter: RBJ LP + limiter] -> [Tube] -> [Compressor] -> mix/volume -> clip -> out
//! ```

mod biquad;
mod compressor;
mod filter;
mod follower;
pub mod params;
pub mod presets;
mod shapers;
mod smoother;

pub use params::{ParamId, NUM_PARAMS};
pub use presets::{Preset, FACTORY_PRESETS};

use compressor::Compressor;
use filter::FilterBank;
use smoother::SmoothedParam;

/// Audio is processed in chunks of at most this many samples.
const CHUNK: usize = 64;
/// Output hard clip.
const CLIP: f32 = f32::from_bits(0x3f7f_f972); // 0.9999
/// Master volume taper: gain = knob^2.737.
const VOLUME_EXP: f32 = 2.737;

/// Release time the main loop hands to the compressor on every call.
const COMP_RELEASE: f32 = f32::from_bits(0x3983_126f); // 0.00025 s
const COMP_MAX_AMOUNT: f32 = f32::from_bits(0x3f7d_70a4); // 0.99

/// Auto-bypass: a block counts as silent when its first and last samples are below this.
const SILENCE_THRESHOLD: f32 = f32::from_bits(0x37fb_a882); // 3e-5
/// Silence that lasts longer than this stops processing (input is passed through).
const SILENCE_SECONDS: f32 = 0.5;

pub struct DuneCrusher {
    params: [SmoothedParam; NUM_PARAMS],
    filter: FilterBank,
    compressor: Compressor,
    /// Set by the Master "On" switch; bypass copies input to output.
    bypassed: bool,
    sample_rate: f32,
    silent_samples: u32,
    mix: f32,
    volume: f32,
    buf_l: [f32; CHUNK],
    buf_r: [f32; CHUNK],
}

impl Default for DuneCrusher {
    fn default() -> Self {
        Self::new()
    }
}

impl DuneCrusher {
    pub fn new() -> Self {
        let mut s = Self {
            params: std::array::from_fn(|_| SmoothedParam::new()),
            filter: FilterBank::new(),
            compressor: Compressor::new(),
            bypassed: false,
            sample_rate: 44100.0,
            silent_samples: 0,
            mix: 0.0,
            volume: 0.0,
            buf_l: [0.0; CHUNK],
            buf_r: [0.0; CHUNK],
        };
        s.set_sample_rate(44100.0);
        s
    }

    pub fn set_sample_rate(&mut self, sr: f32) {
        self.sample_rate = sr;
        self.filter.set_sample_rate(sr);
        self.compressor.set_sample_rate(sr);
        for p in &mut self.params {
            p.set_sample_rate(sr);
        }
    }

    /// Snaps parameter ramps and resets the compressor gain, e.g. when playback starts.
    pub fn reset(&mut self) {
        for p in &mut self.params {
            p.snap();
        }
        self.compressor.reset();
        self.silent_samples = 0;
    }

    /// Sets a normalized (0..1) parameter. Values ramp over 10 ms; the Master On and Phat
    /// switches also act immediately.
    pub fn set_param(&mut self, id: ParamId, value: f32) {
        // Values are rescaled into the parameter's internal range; only CompressAmount has
        // one other than 0..1.
        let internal = if id == ParamId::CompressAmount {
            COMP_MAX_AMOUNT * value
        } else {
            value
        };
        self.params[id as usize].set(internal);
        match id {
            ParamId::CompressMode => self.compressor.set_phat(value >= 0.5),
            ParamId::MasterOn => self.bypassed = value < 0.5,
            _ => {}
        }
    }

    pub fn load_preset(&mut self, preset: &Preset) {
        for (i, &v) in preset.values.iter().enumerate() {
            self.set_param(ParamId::ALL[i], v);
        }
    }

    /// Processes one host block. Input and output slices must all have the same length.
    pub fn process(&mut self, in_l: &[f32], in_r: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        let n = in_l.len();
        debug_assert!(in_r.len() == n && out_l.len() == n && out_r.len() == n);
        if n == 0 {
            return;
        }
        if self.bypassed || self.update_silence(in_l, in_r) {
            out_l.copy_from_slice(in_l);
            out_r.copy_from_slice(in_r);
            return;
        }

        // Module switches are sampled once per host block, and the drive knobs are only
        // read when the distortion module is on. Each read advances that
        // parameter's ramp, so the read pattern is part of the behavior.
        let block = n as i32;
        let dist_on = self.read(ParamId::DistOn, block) > 0.5;
        let mut mech_on = false;
        let mut tube_on = false;
        if dist_on {
            mech_on = self.read(ParamId::DistMech, block) > 0.0;
            tube_on = self.read(ParamId::DistTube, block) > 0.0;
        }
        let filter_on = self.read(ParamId::MmFilterOn, block) > 0.5;
        let comp_on = self.read(ParamId::CompressOn, block) > 0.5;

        let mut pos = 0;
        while pos < n {
            let len = (n - pos).min(CHUNK);
            let c = len as i32;
            let inl = &in_l[pos..pos + len];
            let inr = &in_r[pos..pos + len];
            let l = &mut self.buf_l[..len];
            let r = &mut self.buf_r[..len];
            l.copy_from_slice(inl);
            r.copy_from_slice(inr);

            if mech_on {
                let amt = self.params[ParamId::DistMech as usize].read(c);
                shapers::mech(l, amt);
                shapers::mech(r, amt);
            }
            if filter_on {
                let cutoff = self.params[ParamId::MmFilterCutoff as usize].read(c);
                let res = self.params[ParamId::MmFilterRes as usize].read(c);
                self.filter.process(cutoff, res, l, r);
            }
            if tube_on {
                let amt = self.params[ParamId::DistTube as usize].read(c);
                shapers::tube(l, amt);
                shapers::tube(r, amt);
            }
            if comp_on {
                let amt = self.params[ParamId::CompressAmount as usize]
                    .read(c)
                    .min(COMP_MAX_AMOUNT);
                self.compressor.process(l, r, amt, COMP_RELEASE);
            }

            let mix = self.params[ParamId::MasterMix as usize].read(c);
            let vol = self.params[ParamId::MasterVolume as usize].read(c);
            let inv = 1.0 / len as f32;
            let mix_step = (mix - self.mix) * inv;
            let vol_step = (vol.powf(VOLUME_EXP) - self.volume) * inv;
            for i in 0..len {
                l[i] = ((1.0 - self.mix) * inl[i] + self.mix * l[i]) * self.volume;
                r[i] = ((1.0 - self.mix) * inr[i] + self.mix * r[i]) * self.volume;
                self.volume += vol_step;
                self.mix += mix_step;
            }

            for i in 0..len {
                out_l[pos + i] = l[i].clamp(-CLIP, CLIP);
                out_r[pos + i] = r[i].clamp(-CLIP, CLIP);
            }
            pos += len;
        }
    }

    /// Auto-bypass. Only the first and last sample of
    /// each channel are checked, so a block with silent edges counts as silent. Returns
    /// true once the input has been silent for more than 0.5 s. Processing stops at that
    /// point, so filter and compressor state freeze until audio returns.
    fn update_silence(&mut self, in_l: &[f32], in_r: &[f32]) -> bool {
        let n = in_l.len();
        let quiet = |x: f32| x.abs() < SILENCE_THRESHOLD;
        if quiet(in_l[0]) && quiet(in_r[0]) && quiet(in_l[n - 1]) && quiet(in_r[n - 1]) {
            self.silent_samples = (self.silent_samples + n as u32) & 0x0fff_ffff;
        } else {
            self.silent_samples = 0;
        }
        self.silent_samples > (SILENCE_SECONDS * self.sample_rate) as u32
    }

    fn read(&mut self, id: ParamId, n: i32) -> f32 {
        self.params[id as usize].read(n)
    }
}
