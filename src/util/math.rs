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

pub fn abs_f32(value: f32) -> f32 {
    value.abs()
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
