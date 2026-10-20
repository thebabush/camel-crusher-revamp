//! The two distortion stages. Both are static waveshapers with a pre-gain set by the knob
//! and a knee that maps the pre-gain level back to 1.0.

const MECH_KNEE: f32 = 0.75;
const TUBE_KNEE: f32 = f32::from_bits(0x3f19_999a); // 0.6
const SQRT_HALF: f32 = f32::from_bits(0x3f35_04f7); // 0.707107, slightly off 1/sqrt(2)
const TUBE_SPAN: f32 = f32::from_bits(0x3f4c_cccc); // 0.8 minus one ulp

/// "Mech": gain of 1..11, then a hard knee at 0.75 with a linear segment above it
/// (slope 0.25 / (gain - 0.75)).
pub fn mech(buf: &mut [f32], amount: f32) {
    let gain = amount * 10.0 + 1.0;
    let slope = 0.25 / (gain - MECH_KNEE);
    for x in buf {
        let y = gain * *x;
        *x = if y > MECH_KNEE {
            (y - MECH_KNEE) * slope + MECH_KNEE
        } else if y < -MECH_KNEE {
            -((-y - MECH_KNEE) * slope + MECH_KNEE)
        } else {
            y
        };
    }
}

/// "Tube": gain of 1..5, linear up to 0.6, then a quadratic knee that reaches 1.0 at
/// |y| == gain. Inputs hotter than 0 dBFS fold back down.
pub fn tube(buf: &mut [f32], amount: f32) {
    let gain = amount * 4.0 + 1.0;
    for x in buf {
        let y = gain * *x;
        *x = if y > TUBE_KNEE {
            tube_knee(y, gain)
        } else if y < -TUBE_KNEE {
            -tube_knee(-y, gain)
        } else {
            y
        };
    }
}

#[inline]
fn tube_knee(y: f32, gain: f32) -> f32 {
    let u = (1.0 - (y - TUBE_KNEE) / (gain - TUBE_KNEE)) * SQRT_HALF;
    (0.5 - u * u) * TUBE_SPAN + TUBE_KNEE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knees_reach_unity_at_full_input() {
        for amt in [0.0, 0.3, 1.0] {
            let mut m = [1.0, -1.0];
            mech(&mut m, amt);
            assert!(
                (m[0] - 1.0).abs() < 1e-6 && (m[1] + 1.0).abs() < 1e-6,
                "{m:?}"
            );
            let mut t = [1.0, -1.0];
            tube(&mut t, amt);
            assert!(
                (t[0] - 1.0).abs() < 1e-5 && (t[1] + 1.0).abs() < 1e-5,
                "{t:?}"
            );
        }
    }

    #[test]
    fn linear_below_knee() {
        let mut m = [0.05];
        mech(&mut m, 0.5);
        assert_eq!(m[0], 0.05 * 6.0);
    }
}
