// --------------------------------------------------------------------------------------------------------------------
// xBR 2x, Hyllian's pixel art scaler, as xBRjs implements it.
//
// A line for line port of the 2x path of reversing/web/vendor/xBRjs.umd.js (Josep del Rio,
// MIT, commit f8b3ab8), which the asset browser draws with. Ported rather than written from
// the algorithm's description on purpose: the provenance note beside that file records an
// in-house kernel being thrown out because its blending came out wrong, and the test here
// holds this port to the library's own output byte for byte (test_data/xbr2x).
//
// Two things in the source are quirks and are kept because matching it is the point:
//   - one difference term in the edge weight is called without scaleAlpha, so it treats any
//     non-opaque pixel as maximally different whatever the caller asked for
//   - the shifts operate on JavaScript numbers, so a floating point difference is truncated
//     to int32 before it is doubled or quadrupled
// Pixels are u32, red in the low byte and alpha in the high one: RGBA bytes read little endian.

const THRESHOLD_Y: f64 = 48.0;
const THRESHOLD_U: f64 = 7.0;
const THRESHOLD_V: f64 = 6.0;
/// The library's "not attainable by the thresholds" difference.
const FAR: f64 = 1_000_000.0;

fn alpha(p: u32) -> u32 {
    (p >> 24) & 0xff
}

fn yuv(p: u32) -> (f64, f64, f64) {
    let r = (p & 0xff) as f64;
    let g = ((p >> 8) & 0xff) as f64;
    let b = ((p >> 16) & 0xff) as f64;
    (r * 0.299 + g * 0.587 + b * 0.114, r * -0.168736 + g * -0.331264 + b * 0.5, r * 0.5 + g * -0.418688 + b * -0.081312)
}

fn yuv_difference(a: u32, b: u32, scale_alpha: bool) -> f64 {
    let (alpha_a, alpha_b) = (alpha(a), alpha(b));
    if alpha_a == 0 && alpha_b == 0 {
        return 0.0;
    }
    if !scale_alpha && (alpha_a < 255 || alpha_b < 255) {
        return FAR;
    }
    if alpha_a == 0 || alpha_b == 0 {
        return FAR;
    }
    let (ya, ua, va) = yuv(a);
    let (yb, ub, vb) = yuv(b);
    (ya - yb).abs() * THRESHOLD_Y + (ua - ub).abs() * THRESHOLD_U + (va - vb).abs() * THRESHOLD_V
}

fn is_equal(a: u32, b: u32, scale_alpha: bool) -> bool {
    let (alpha_a, alpha_b) = (alpha(a), alpha(b));
    if alpha_a == 0 && alpha_b == 0 {
        return true;
    }
    if !scale_alpha && (alpha_a < 255 || alpha_b < 255) {
        return false;
    }
    if alpha_a == 0 || alpha_b == 0 {
        return false;
    }
    let (ya, ua, va) = yuv(a);
    let (yb, ub, vb) = yuv(b);
    (ya - yb).abs() <= THRESHOLD_Y && (ua - ub).abs() <= THRESHOLD_U && (va - vb).abs() <= THRESHOLD_V
}

/// JavaScript's x << n on a number: truncate to int32 first.
fn shifted(x: f64, n: u32) -> f64 {
    ((x as i32) << n) as f64
}

/// Blend a towards b in the ratio q1 to q2, a fully transparent side contributing no colour.
fn interpolate(a: u32, b: u32, q1: u32, q2: u32) -> u32 {
    let (alpha_a, alpha_b) = (alpha(a), alpha(b));
    let channel = |p: u32, shift: u32| ((p >> shift) & 0xff) as f64;
    let (q1f, q2f) = (q1 as f64, q2 as f64);
    let mix = |shift| (q2f * channel(b, shift) + q1f * channel(a, shift)) / (q1f + q2f);
    let (r, g, bl) = if alpha_a == 0 {
        (channel(b, 0), channel(b, 8), channel(b, 16))
    } else if alpha_b == 0 {
        (channel(a, 0), channel(a, 8), channel(a, 16))
    } else {
        (mix(0), mix(8), mix(16))
    };
    let al = (q2f * alpha_b as f64 + q1f * alpha_a as f64) / (q1f + q2f);
    (r as u32) | ((g as u32) << 8) | ((bl as u32) << 16) | ((al as u32) << 24)
}

fn blend_64(dst: u32, src: u32, blend: bool) -> u32 {
    if blend { interpolate(dst, src, 3, 1) } else { dst }
}

fn blend_128(dst: u32, src: u32, blend: bool) -> u32 {
    if blend { interpolate(dst, src, 1, 1) } else { dst }
}

fn blend_192(dst: u32, src: u32, blend: bool) -> u32 {
    if blend { interpolate(dst, src, 1, 3) } else { src }
}

/// The 21 neighbours the kernel looks at, edges clamped. Named as the library names them.
struct Neighbours {
    a1: u32, b1: u32, c1: u32,
    a0: u32, pa: u32, pb: u32, pc: u32, c4: u32,
    d0: u32, pd: u32, pe: u32, pf: u32, f4: u32,
    g0: u32, pg: u32, ph: u32, pi: u32, i4: u32,
    g5: u32, h5: u32, i5: u32,
}

fn neighbours(src: &[u32], x: usize, y: usize, w: usize, h: usize) -> Neighbours {
    let xm1 = x.saturating_sub(1);
    let xm2 = x.saturating_sub(2);
    let xp1 = (x + 1).min(w - 1);
    let xp2 = (x + 2).min(w - 1);
    let ym1 = y.saturating_sub(1);
    let ym2 = y.saturating_sub(2);
    let yp1 = (y + 1).min(h - 1);
    let yp2 = (y + 2).min(h - 1);
    let at = |col: usize, row: usize| src[col + row * w];
    Neighbours {
        a1: at(xm1, ym2), b1: at(x, ym2), c1: at(xp1, ym2),
        a0: at(xm2, ym1), pa: at(xm1, ym1), pb: at(x, ym1), pc: at(xp1, ym1), c4: at(xp2, ym1),
        d0: at(xm2, y), pd: at(xm1, y), pe: at(x, y), pf: at(xp1, y), f4: at(xp2, y),
        g0: at(xm2, yp1), pg: at(xm1, yp1), ph: at(x, yp1), pi: at(xp1, yp1), i4: at(xp2, yp1),
        g5: at(xm1, yp2), h5: at(x, yp2), i5: at(xp1, yp2),
    }
}

/// One corner of the 2x2 output, the library's kernel2Xv5. Returns the three output pixels
/// it may have touched, in the order it was given them.
#[allow(clippy::too_many_arguments)]
fn kernel(pe: u32, pi: u32, ph: u32, pf: u32, pg: u32, pc: u32, pd: u32, pb: u32, f4: u32, i4: u32, h5: u32, i5: u32, n1: u32, n2: u32, n3: u32, blend: bool, scale_alpha: bool) -> (u32, u32, u32) {
    let (mut n1, mut n2, mut n3) = (n1, n2, n3);
    if pe == ph || pe == pf {
        return (n1, n2, n3);
    }
    let d = |a, b| yuv_difference(a, b, scale_alpha);
    let eq = |a, b| is_equal(a, b, scale_alpha);
    // The quirk: the library leaves scaleAlpha off this one term.
    let e = d(pe, pc) + d(pe, pg) + d(pi, h5) + d(pi, f4) + shifted(yuv_difference(ph, pf, false), 2);
    let i = d(ph, pd) + d(ph, i5) + d(pf, i4) + d(pf, pb) + shifted(d(pe, pi), 2);
    let px = if d(pe, pf) <= d(pe, ph) { pf } else { ph };
    let edge = (!eq(pf, pb) && !eq(ph, pd)) || (eq(pe, pi) && !eq(pf, i4) && !eq(ph, i5)) || eq(pe, pg) || eq(pe, pc);
    if e < i && edge {
        let ke = d(pf, pg);
        let ki = d(ph, pc);
        let ex2 = pe != pc && pb != pc;
        let ex3 = pe != pg && pd != pg;
        let left = shifted(ke, 1) <= ki && ex3;
        let up = ke >= shifted(ki, 1) && ex2;
        if left || up {
            if left {
                n3 = blend_192(n3, px, blend);
                n2 = blend_64(n2, px, blend);
            }
            if up {
                n3 = blend_192(n3, px, blend);
                n1 = blend_64(n1, px, blend);
            }
        } else {
            n3 = blend_128(n3, px, blend);
        }
    } else if e <= i {
        n3 = blend_64(n3, px, blend);
    }
    (n1, n2, n3)
}

/// Scale by two. blend_colours off gives hard edges from the source colours only;
/// scale_alpha on lets the filter work across a sprite's transparent surround, which is how
/// the asset browser calls it. The output is width * 2 by height * 2.
pub fn xbr2x(src: &[u32], width: usize, height: usize, blend_colours: bool, scale_alpha: bool) -> Vec<u32> {
    let dst_w = width * 2;
    let mut dst = vec![0u32; dst_w * height * 2];
    if width == 0 || height == 0 {
        return dst;
    }
    for y in 0..height {
        for x in 0..width {
            let n = neighbours(src, x, y, width, height);
            let (mut e0, mut e1, mut e2, mut e3) = (n.pe, n.pe, n.pe, n.pe);
            (e1, e2, e3) = kernel(n.pe, n.pi, n.ph, n.pf, n.pg, n.pc, n.pd, n.pb, n.f4, n.i4, n.h5, n.i5, e1, e2, e3, blend_colours, scale_alpha);
            (e0, e3, e1) = kernel(n.pe, n.pc, n.pf, n.pb, n.pi, n.pa, n.ph, n.pd, n.b1, n.c1, n.f4, n.c4, e0, e3, e1, blend_colours, scale_alpha);
            (e2, e1, e0) = kernel(n.pe, n.pa, n.pb, n.pd, n.pc, n.pg, n.pf, n.ph, n.d0, n.a0, n.b1, n.a1, e2, e1, e0, blend_colours, scale_alpha);
            (e3, e0, e2) = kernel(n.pe, n.pg, n.pd, n.ph, n.pa, n.pi, n.pb, n.pf, n.h5, n.g5, n.d0, n.g0, e3, e0, e2, blend_colours, scale_alpha);
            let (dx, dy) = (x * 2, y * 2);
            dst[dx + dy * dst_w] = e0;
            dst[dx + 1 + dy * dst_w] = e1;
            dst[dx + (dy + 1) * dst_w] = e2;
            dst[dx + 1 + (dy + 1) * dst_w] = e3;
        }
    }
    dst
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    const INPUT: &[u8] = include_bytes!("../../../test_data/xbr2x/input.rgba");
    const EXPECTED: &[u8] = include_bytes!("../../../test_data/xbr2x/expected.rgba");

    fn as_u32(bytes: &[u8]) -> Vec<u32> {
        bytes.chunks(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
    }

    #[test]
    fn the_port_matches_xbrjs_byte_for_byte() {
        let out = xbr2x(&as_u32(INPUT), 16, 12, true, true);
        let expected = as_u32(EXPECTED);
        assert_eq!(out.len(), expected.len());
        let first_mismatch = out.iter().zip(&expected).position(|(a, b)| a != b);
        assert_eq!(first_mismatch, None, "output pixel {:?} differs: {:08x} vs {:08x}", first_mismatch, first_mismatch.map(|i| out[i]).unwrap_or(0), first_mismatch.map(|i| expected[i]).unwrap_or(0));
    }

    #[test]
    fn a_flat_image_stays_flat_and_an_empty_one_is_empty() {
        let flat = vec![0xff00_80ffu32; 6];
        assert!(xbr2x(&flat, 3, 2, true, true).iter().all(|p| *p == 0xff00_80ff));
        assert!(xbr2x(&[], 0, 0, true, true).is_empty());
    }

    #[test]
    fn interpolation_follows_the_library() {
        assert_eq!(interpolate(0xff00_0000, 0xff00_00ff, 1, 1), 0xff00_007f, "halfway, truncated");
        assert_eq!(interpolate(0x0000_0000, 0xff11_2233, 3, 1), 0x3f11_2233, "a transparent side lends only alpha");
        assert_eq!(shifted(2.9, 1), 4.0);
    }
}
