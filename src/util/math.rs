pub fn floor_double(value: f64) -> i32 {
    let mut truncated = value as i32;
    if value < truncated as f64 {
        truncated -= 1;
    }
    truncated
}

pub fn floor_float(value: f32) -> i32 {
    let mut truncated = value as i32;
    if value < truncated as f32 {
        truncated -= 1;
    }
    truncated
}

pub fn floor_div(a: i32, b: i32) -> i32 {
    let quotient = a / b;
    if a % b != 0 && (a ^ b) < 0 {
        quotient - 1
    } else {
        quotient
    }
}

pub fn floor_mod(a: i32, b: i32) -> i32 {
    a - floor_div(a, b) * b
}

pub fn clamp_i32(value: i32, min: i32, max: i32) -> i32 {
    value.max(min).min(max)
}

pub fn lerp(delta: f64, start: f64, end: f64) -> f64 {
    start + delta * (end - start)
}

pub fn sqrt_f32(value: f32) -> f32 {
    value.sqrt()
}

/// Java `MathHelper.a(double)`: the square root of a double, rounded to a
/// single precision value.
///
/// The reference takes the square root in the double precision and rounds
/// the result, the single precision square root can differ in the last bit.
pub fn math_helper_a(value: f64) -> f32 {
    value.sqrt() as f32
}

pub fn abs_f32(value: f32) -> f32 {
    value.abs()
}

/// The trigonometry of the reference (`org.bukkit.craftbukkit.TrigMath`).
///
/// The reference uses its own implementation of the arcus tangens (a
/// polynomial approximation of the fdlibm), the function of the standard
/// library would differ in the last bits of the result.
#[allow(clippy::approx_constant)]
pub mod trig {
    const P4: f64 = 16.153_641_298_223_02;
    const P3: f64 = 268.425_481_955_039_74;
    const P2: f64 = 1_153.029_351_540_485;
    const P1: f64 = 1_780.406_316_433_197;
    const P0: f64 = 896.785_974_036_638_7;
    const Q4: f64 = 58.956_970_508_444_62;
    const Q3: f64 = 536.265_374_031_215_3;
    const Q2: f64 = 1_666.783_814_881_633_8;
    const Q1: f64 = 2_079.334_974_445_41;
    const Q0: f64 = 896.785_974_036_638_7;
    const SQ2P1: f64 = 2.414_213_562_373_095;
    const SQ2M1: f64 = 0.414_213_562_373_095_03;
    const PIO2: f64 = 1.570_796_326_794_896_6;
    const PI: f64 = 3.141_592_653_589_793;

    /// `TrigMath.mxatan(double)`.
    fn mxatan(x: f64) -> f64 {
        let x2 = x * x;
        let p = (((P4 * x2 + P3) * x2 + P2) * x2 + P1) * x2 + P0;
        let q = ((((Q4 + x2) * x2 + Q3) * x2 + Q2) * x2 + Q1) * x2 + Q0;
        (p / q) * x
    }

    /// `TrigMath.msatan(double)`.
    fn msatan(x: f64) -> f64 {
        if x < SQ2M1 {
            mxatan(x)
        } else if x > SQ2P1 {
            PIO2 - mxatan(1.0 / x)
        } else {
            0.785_398_163_397_448_3 + mxatan((x - 1.0) / (x + 1.0))
        }
    }

    /// `TrigMath.atan(double)`.
    pub fn atan(x: f64) -> f64 {
        if x > 0.0 {
            msatan(x)
        } else {
            -msatan(-x)
        }
    }

    /// `TrigMath.atan2(double, double)`.
    pub fn atan2(y: f64, x: f64) -> f64 {
        if x == 0.0 {
            return if y < 0.0 { -PIO2 } else { PIO2 };
        }
        let angle = atan(y / x);
        if x >= 0.0 {
            angle
        } else if angle > 0.0 {
            angle - PI
        } else {
            angle + PI
        }
    }
}

static SIN_TABLE: std::sync::OnceLock<[f32; 65536]> = std::sync::OnceLock::new();

fn sin_table() -> &'static [f32; 65536] {
    SIN_TABLE.get_or_init(|| {
        let mut table = [0.0f32; 65536];
        for (index, slot) in table.iter_mut().enumerate() {
            *slot = ((index as f64) * std::f64::consts::PI * 2.0 / 65536.0).sin() as f32;
        }
        table
    })
}

pub fn sin(value: f32) -> f32 {
    sin_table()[(value * 10430.378f32) as i32 as usize & 0xffff]
}

pub fn cos(value: f32) -> f32 {
    sin_table()[(value * 10430.378f32 + 16384.0f32) as i32 as usize & 0xffff]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[allow(clippy::excessive_precision, clippy::approx_constant)]
    fn trig_matches_the_reference() {
        let cases = [
            (0.0f64, 2.0f64),
            (-1.0, -2.0),
            (1.0, 1.0),
            (-1.0, 1.0),
            (0.5, -0.5),
            (-2.0, 0.3),
            (0.0, -1.0),
            (1.0, 0.0),
        ];
        let expected = [
            0.0f64,
            -2.6779450445889870,
            0.78539816339744830,
            -0.78539816339744830,
            2.3561944901923450,
            -1.4219063791853994,
            3.1415926535897930,
            1.5707963267948966,
        ];
        for (i, (y, x)) in cases.iter().enumerate() {
            let got = crate::util::math::trig::atan2(*y, *x);
            assert_eq!(
                got.to_bits(),
                expected[i].to_bits(),
                "atan2({y}, {x}) = {got} != {}",
                expected[i]
            );
        }
    }

    #[test]
    fn floors_toward_negative_infinity() {
        assert_eq!(floor_double(1.9), 1);
        assert_eq!(floor_double(-1.1), -2);
        assert_eq!(floor_float(2.0), 2);
        assert_eq!(floor_float(-0.5), -1);
    }

    #[test]
    fn floored_division_matches_vanilla() {
        assert_eq!(floor_div(7, 3), 2);
        assert_eq!(floor_div(-7, 3), -3);
        assert_eq!(floor_div(7, -3), -3);
        assert_eq!(floor_div(-7, -3), 2);
        assert_eq!(floor_mod(-7, 3), 2);
        assert_eq!(floor_mod(7, -3), -2);
    }
}
