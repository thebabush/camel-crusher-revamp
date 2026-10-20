//! The filter stage ("LP2 Fat"): one RBJ low-pass biquad per channel, no oversampling, an
//! output limiter, and a dry/wet crossfade that is pinned to fully wet.
//!
//! The cutoff knob (0..1) maps to Hz as `80 + 15920 * clamp(-log10(1 - v) / 4, 0, 1)`. It is
//! then converted to a MIDI note and back before reaching the biquad. Cutoff changes are
//! ramped linearly across the host chunk and pushed to the biquads every 16 samples.

use crate::biquad::Biquad;
use crate::follower::Follower;

const SUB_BLOCK: usize = 16;
const LIMITER_RECOVERY_SECONDS: f32 = 7.5;
const MAX_CHUNK: usize = 64;

pub struct FilterBank {
    left: Biquad,
    right: Biquad,
    initialized: bool,
    sample_rate: f32,
    max_note: f32,
    // All of these start at -1 after a sample-rate change, which causes a short sweep up
    // from 80 Hz and a -1..1 wet ramp on the first chunk. That onset is part of the sound
    // and is kept.
    cutoff: f32,
    resonance: f32,
    env_amount: f32,
    wet: f32,
    limiter_env: Follower,
    limiter_gain: f32,
    limiter_step: f32,
    dry_l: [f32; MAX_CHUNK],
    dry_r: [f32; MAX_CHUNK],
}

/// Cutoff knob to Hz, as displayed and fed to the filter.
pub fn cutoff_to_hz(v: f32) -> f32 {
    if v >= 1.0 {
        16000.0
    } else if v <= 0.0 {
        80.0
    } else {
        let mut t = ((1.0 - v).log10() * -0.25) as f64;
        if t > 1.0 {
            t = 1.0;
        } else if t < 1e-7 {
            t = 0.0;
        }
        (t * 15920.0 + 80.0) as f32
    }
}

fn hz_to_note(hz: f32) -> f32 {
    (12.0 / 2f32.ln()) * (hz * f32::from_bits(0x3b14_f209)).ln() + 69.0
}

impl FilterBank {
    pub fn new() -> Self {
        let mut s = Self {
            left: Biquad::new(),
            right: Biquad::new(),
            initialized: false,
            sample_rate: 44100.0,
            max_note: 0.0,
            cutoff: -1.0,
            resonance: -1.0,
            env_amount: -1.0,
            wet: -1.0,
            limiter_env: Follower::new(
                f32::from_bits(0x3bf5_c28f), // 0.0075
                f32::from_bits(0x3d4c_cccd), // 0.05
            ),
            limiter_gain: 1.0,
            limiter_step: 0.0,
            dry_l: [0.0; MAX_CHUNK],
            dry_r: [0.0; MAX_CHUNK],
        };
        s.set_sample_rate(44100.0);
        s
    }

    pub fn set_sample_rate(&mut self, sr: f32) {
        self.sample_rate = sr;
        self.limiter_gain = 1.0;
        self.limiter_step = 0.0;
        self.limiter_env.set_sample_rate(sr);
        self.initialized = false;
        self.cutoff = -1.0;
        self.resonance = -1.0;
        self.env_amount = -1.0;
        self.wet = -1.0;
        let nyquist_note =
            ((sr * 0.5) * f32::from_bits(0x3b14_f209)).ln() * (12.0 / 2f32.ln()) + 69.0;
        self.max_note = nyquist_note.max(0.0) - 0.01;
    }

    fn note_for(&self, cutoff: f32) -> f32 {
        // Envelope modulation is wired but always 0 here, so the env term
        // drops out; `(1 - env_amount)` still matters while env_amount is at its -1 init.
        let v = (1.0 - self.env_amount) * cutoff;
        hz_to_note(cutoff_to_hz(v)).max(0.0).min(self.max_note)
    }

    /// Filters one chunk (at most 64 samples) in place.
    pub fn process(&mut self, cutoff: f32, resonance: f32, l: &mut [f32], r: &mut [f32]) {
        let n = l.len();
        const WET: f32 = 1.0;
        const ENV_AMOUNT: f32 = 0.0;
        let wet_step = (WET - self.wet) / n as f32;
        self.dry_l[..n].copy_from_slice(l);
        self.dry_r[..n].copy_from_slice(r);

        let mut cutoff_step = 0.0;
        let ramping = cutoff != self.cutoff || !self.initialized;
        if ramping {
            if !self.initialized {
                self.initialized = true;
                self.limiter_gain = 1.0;
                self.limiter_step = 0.0;
                self.limiter_env.clear();
                self.left.set_sample_rate(self.sample_rate);
                self.right.set_sample_rate(self.sample_rate);
            }
            cutoff_step = (cutoff - self.cutoff) / n as f32;
        }

        if self.env_amount != 0.0 || self.resonance != resonance || self.env_amount != ENV_AMOUNT {
            self.resonance = resonance;
            self.env_amount = ENV_AMOUNT;
            let note = self.note_for(self.cutoff);
            self.left.set_resonance(resonance);
            self.right.set_resonance(resonance);
            self.left.set_cutoff_note(note);
            self.right.set_cutoff_note(note);
        }

        let mut pos = 0;
        while pos < n {
            let len = (n - pos).min(SUB_BLOCK);
            self.left.process(&mut l[pos..pos + len]);
            self.right.process(&mut r[pos..pos + len]);
            if ramping {
                self.cutoff += len as f32 * cutoff_step;
                let note = self.note_for(self.cutoff);
                self.left.set_cutoff_note(note);
                self.right.set_cutoff_note(note);
            }
            pos += len;
        }
        self.cutoff = cutoff;
        self.resonance = resonance;

        let recovery = self.sample_rate * LIMITER_RECOVERY_SECONDS;
        for i in 0..n {
            let al = l[i].abs();
            let ar = r[i].abs();
            let peak = if al > ar { al } else { ar };
            let env = self.limiter_env.process(peak);
            if env > 1.0 {
                let g = 1.0 / env;
                if self.limiter_gain > g {
                    self.limiter_gain = g;
                    self.limiter_step = (1.0 - g) / recovery;
                }
            }
            if 1.0 > self.limiter_gain {
                r[i] *= self.limiter_gain;
                l[i] *= self.limiter_gain;
                self.limiter_gain += self.limiter_step;
            }
        }

        for i in 0..n {
            l[i] = (1.0 - self.wet) * self.dry_l[i] + self.wet * l[i];
            r[i] = (1.0 - self.wet) * self.dry_r[i] + self.wet * r[i];
            self.wet += wet_step;
        }
        self.wet = WET;
    }
}
