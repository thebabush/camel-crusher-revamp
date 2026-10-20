//! A feed-forward peak compressor with instant attack, linear gain recovery, and a fixed
//! soft knee at 0.31.
//!
//! The Amount knob is really a make-up gain of `1 / (1 - amount)` applied before the
//! detector, so "compression" is the signal being pushed into the fixed knee. The knee maps
//! detector levels in `[0.31, 1/(1-amount)]` onto `[0.31, 1.0]` along a quadratic curve.
//!
//! Phat on: the detector is the raw stereo peak of each sample, and gain recovers within
//! 0.25 ms, so the compressor acts mostly as a waveshaper. That is the manual's "crackle".
//! Phat off: the detector is a 30 ms / 80 ms linear follower, which acts as a real
//! compressor.

use crate::follower::Follower;

const KNEE: f32 = f32::from_bits(0x3e9e_b852); // 0.31
const KNEE_SPAN: f32 = f32::from_bits(0x3fb0_a3d7); // 1.38
const SQRT_HALF: f32 = f32::from_bits(0x3f35_04f7);
const SMOOTH_MODE: f32 = f32::from_bits(0x3c23_d70a); // 0.01: any nonzero value selects the follower

pub struct Compressor {
    amount: f32,
    mode: f32,
    release: f32,
    sample_rate: f32,
    gain: f32,
    gain_step: f32,
    follower: Follower,
}

impl Compressor {
    pub fn new() -> Self {
        Self {
            amount: 0.0,
            mode: 0.0,
            // Zero until the first process() call stores the real value, so the first chunk
            // after instantiation recovers gain immediately (step = +inf).
            release: 0.0,
            sample_rate: 44100.0,
            gain: 1.0,
            gain_step: 0.0,
            follower: Follower::new(
                f32::from_bits(0x3cf5_c28f), // 0.03
                f32::from_bits(0x3da3_d70a), // 0.08
            ),
        }
    }

    pub fn set_sample_rate(&mut self, sr: f32) {
        self.sample_rate = sr;
        self.follower.set_sample_rate(sr);
    }

    pub fn reset(&mut self) {
        self.gain = 1.0;
        self.gain_step = 0.0;
        self.follower.clear();
    }

    pub fn set_phat(&mut self, phat: bool) {
        self.mode = if phat { 0.0 } else { SMOOTH_MODE };
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], amount: f32, release: f32) {
        let n = l.len();
        // Makeup and release use the values from the previous call; the amount ramps
        // only for bookkeeping.
        let release_samples = self.sample_rate * self.release;
        let makeup = 1.0 / (1.0 - self.amount);
        let amount_step = (amount - self.amount) / n as f32;
        self.release = release;

        for i in 0..n {
            let mut xl = l[i];
            let mut xr = r[i];
            let al = xl.abs();
            let ar = xr.abs();
            let peak = if al > ar { al } else { ar };
            let det = if self.mode == 0.0 {
                peak
            } else {
                self.follower.process(peak)
            };
            let level = det * makeup;
            xl *= makeup;
            xr *= makeup;
            if level > KNEE {
                let u = (1.0 - (level - KNEE) / (makeup - KNEE)) * SQRT_HALF;
                let target = ((0.5 - u * u) * KNEE_SPAN + KNEE) / level;
                if self.gain > target {
                    self.gain = target;
                    self.gain_step = (1.0 - target) / release_samples;
                }
            }
            if 1.0 > self.gain {
                xl *= self.gain;
                xr *= self.gain;
                self.gain += self.gain_step;
            }
            l[i] = xl;
            r[i] = xr;
            self.amount += amount_step;
        }
        self.amount = amount;
    }
}
