//! Linear-slope peak follower, used by the compressor and the filter limiter. Rises by at
//! most `1 / (attack * sr)` per sample and decays by `1 / (release * sr)` per sample, i.e.
//! full-scale in `attack` / `release` seconds.

pub struct Follower {
    attack: f32,
    release: f32,
    env: f32,
    attack_step: f32,
    release_step: f32,
}

impl Follower {
    pub fn new(attack: f32, release: f32) -> Self {
        let mut f = Self {
            attack,
            release,
            env: 0.0,
            attack_step: 0.0,
            release_step: 0.0,
        };
        f.set_sample_rate(44100.0);
        f
    }

    pub fn set_sample_rate(&mut self, sr: f32) {
        self.attack_step = if self.attack == 0.0 {
            0.0
        } else {
            1.0 / (self.attack * sr)
        };
        self.release_step = if self.release == 0.0 {
            0.0
        } else {
            1.0 / (self.release * sr)
        };
        self.env = 0.0;
    }

    pub fn clear(&mut self) {
        self.env = 0.0;
    }

    pub fn process(&mut self, x: f32) -> f32 {
        let e = self.env - self.release_step;
        let mut d = if x > e {
            x - e
        } else if -e > x {
            -x - e
        } else {
            0.0
        };
        if d > self.attack_step {
            d = self.attack_step;
        }
        let e = e + d;
        self.env = if e >= 0.0 { e } else { 0.0 };
        self.env
    }
}
