// --------------------------------------------------------------------------------------------------------------------
// Value noise, for the custom generator and for the detail the high resolution DEM adds.
//
// Plain value noise on an integer lattice with smooth interpolation, summed over octaves.
// Deterministic for a seed, no tables, no dependencies. It does not need to be pretty; it
// needs to be repeatable and cheap.

/// A well mixed 32-bit hash of a lattice point and a seed, in 0.0 ..= 1.0.
pub fn lattice_value(ix: i32, iy: i32, seed: u64) -> f32 {
    let mut h = (ix as u32 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (iy as u32 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ seed.wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 31;
    h = h.wrapping_mul(0x7FB5_D329_728E_A185);
    h ^= h >> 27;
    h = h.wrapping_mul(0x81DA_DEF4_BC2D_D44D);
    h ^= h >> 33;
    (h & 0x00FF_FFFF) as f32 / 0x00FF_FFFF as f32
}

/// Smoothstep, the interpolation curve between lattice points.
fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// One octave of value noise at a point, in 0.0 ..= 1.0.
pub fn value_noise(x: f32, y: f32, seed: u64) -> f32 {
    let x0 = x.floor();
    let y0 = y.floor();
    let tx = smooth(x - x0);
    let ty = smooth(y - y0);
    let (ix, iy) = (x0 as i32, y0 as i32);
    let v00 = lattice_value(ix, iy, seed);
    let v10 = lattice_value(ix + 1, iy, seed);
    let v01 = lattice_value(ix, iy + 1, seed);
    let v11 = lattice_value(ix + 1, iy + 1, seed);
    let top = v00 + (v10 - v00) * tx;
    let bottom = v01 + (v11 - v01) * tx;
    top + (bottom - top) * ty
}

/// Fractal (Brownian) noise: octaves of value noise, each at twice the frequency and `gain`
/// of the amplitude of the last, normalised back into 0.0 ..= 1.0.
pub fn fbm(x: f32, y: f32, seed: u64, octaves: u32, gain: f32) -> f32 {
    let mut total = 0.0;
    let mut amplitude = 1.0;
    let mut frequency = 1.0;
    let mut norm = 0.0;
    for octave in 0..octaves.max(1) {
        total += value_noise(x * frequency, y * frequency, seed.wrapping_add(octave as u64 * 0x9E37)) * amplitude;
        norm += amplitude;
        amplitude *= gain;
        frequency *= 2.0;
    }
    total / norm
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_deterministic_and_bounded() {
        for i in 0..200 {
            let x = i as f32 * 0.37;
            let y = i as f32 * 0.11;
            let a = fbm(x, y, 42, 5, 0.5);
            let b = fbm(x, y, 42, 5, 0.5);
            assert_eq!(a, b);
            assert!((0.0..=1.0).contains(&a), "{a} out of range");
        }
    }

    #[test]
    fn seeds_differ() {
        let a: Vec<f32> = (0..50).map(|i| value_noise(i as f32 * 0.7, 1.5, 1)).collect();
        let b: Vec<f32> = (0..50).map(|i| value_noise(i as f32 * 0.7, 1.5, 2)).collect();
        assert_ne!(a, b);
    }

    #[test]
    fn noise_hits_lattice_values_exactly() {
        assert_eq!(value_noise(3.0, 4.0, 9), lattice_value(3, 4, 9));
    }
}
