//! Parameter value ramping.
//!
//! A new target is reached linearly over 10 ms. The ramp advances by however many samples
//! the caller says it is about to process, once per read.

const RAMP_SECONDS: f32 = f32::from_bits(0x3c23_d70a); // 0.01

pub struct SmoothedParam {
    start: f32,
    current: f32,
    target: f32,
    counter: i32,
    ramp_len: i32,
}

impl SmoothedParam {
    pub fn new() -> Self {
        Self {
            start: 0.0,
            current: 0.0,
            target: 0.0,
            counter: 0,
            ramp_len: 0,
        }
    }

    pub fn set_sample_rate(&mut self, sr: f32) {
        self.ramp_len = (sr * RAMP_SECONDS) as i32;
    }

    pub fn set(&mut self, value: f32) {
        if self.target != value {
            self.target = value;
            self.counter = 0;
            self.start = self.current;
        }
    }

    pub fn snap(&mut self) {
        self.current = self.target;
        self.start = 0.0;
        self.counter = 0;
    }

    pub fn read(&mut self, n: i32) -> f32 {
        if self.current != self.target {
            self.counter += n;
            let mut f = if self.ramp_len > 0 {
                self.counter as f32 / self.ramp_len as f32
            } else {
                1.0
            };
            if f > 1.0 {
                f = 1.0;
            }
            self.current = (1.0 - f) * self.start + self.target * f;
        }
        self.current
    }
}
