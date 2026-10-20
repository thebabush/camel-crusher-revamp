//! Low-pass biquad: RBJ cookbook LPF, direct form I, single precision
//! state, coefficients designed in double precision.

pub struct Biquad {
    sample_rate: f32,
    cutoff_hz: f32,
    q: f32,
    dirty: bool,
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

#[inline]
fn flush(x: f32) -> f32 {
    if x.is_subnormal() {
        0.0
    } else {
        x
    }
}

impl Biquad {
    pub fn new() -> Self {
        Self {
            sample_rate: 44100.0,
            cutoff_hz: 10000.0,
            q: 0.0,
            dirty: true,
            b0: 0.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    pub fn set_sample_rate(&mut self, sr: f32) {
        self.sample_rate = sr;
        self.reset();
    }

    pub fn reset(&mut self) {
        self.dirty = true;
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }

    /// Cutoff as a MIDI note number (69 = 440 Hz), which is how the filter bank sets it.
    pub fn set_cutoff_note(&mut self, note: f32) {
        self.cutoff_hz = 2f32.powf((note - 69.0) * f32::from_bits(0x3daa_aaab)) * 440.0;
        self.dirty = true;
    }

    /// Resonance knob 0..1, mapped to Q = 1 + 6 * res.
    pub fn set_resonance(&mut self, res: f32) {
        self.q = res * 6.0 + 1.0;
        self.dirty = true;
    }

    fn design(&mut self) {
        let w = (self.cutoff_hz / self.sample_rate) as f64 * std::f64::consts::TAU;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / (self.q as f64 * 2.0);
        let norm = alpha + 1.0;
        let b0 = ((1.0 - cos) * 0.5 / norm) as f32;
        self.b0 = flush(b0);
        self.b1 = flush(((1.0 - cos) / norm) as f32);
        self.b2 = flush(b0);
        self.a1 = flush((cos * -2.0 / norm) as f32);
        self.a2 = flush(((1.0 - alpha) / norm) as f32);
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        if self.dirty {
            self.dirty = false;
            self.design();
        }
        for x in buf {
            let y = ((*x * self.b0 + self.x1 * self.b1) + self.b2 * self.x2)
                - self.y1 * self.a1
                - self.a2 * self.y2;
            self.x2 = self.x1;
            self.x1 = *x;
            self.y2 = self.y1;
            self.y1 = y;
            *x = y;
        }
        // Blow-up / denormal guard, once per call.
        if self.y1.is_subnormal()
            || !(-1000.0..=1000.0).contains(&self.y1)
            || !(-1000.0..=1000.0).contains(&self.y2)
        {
            self.y1 = 0.0;
            self.y2 = 0.0;
        }
    }
}
