// --------------------------------------------------------------------------------------------------------------------
// Turning the live values into the bytes of a uniform block.
//
// The editor owns the buffer, so this is where it agrees with the GPU: every leaf writes its
// components at the offset std140.rs worked out. A resolution or texel member is bound from
// the render size rather than from the stored value, so a bake at a new size is correct
// without anyone touching a control, which is the same deal the paired path gives
// format=resolution.

use macroquad::prelude::*;

use crate::glsl_meta::{matches_fmt, GLSLType, UniformSpec};
use crate::unified::parse::{Block, Unified};
use crate::unified::std140::Leaf;
use crate::values::UniformValues;

/// Every leaf as a UniformSpec, so the unified path reuses the defaults, the --set parsing and
/// the motion controls that the paired path already has.
pub fn leaf_specs(shader: &Unified) -> Vec<UniformSpec> {
    shader.leaves()
        .map(|(_, l)| UniformSpec { name: l.key.clone(), ty: l.ty, hint: l.hint.clone() })
        .collect()
}

/// The value a member is bound to by the render size, if its format says it is one.
fn bound_to_size(leaf: &Leaf, size: Vec2) -> Option<Vec4> {
    let texel = vec2(1.0 / size.x.max(1.0), 1.0 / size.y.max(1.0));
    match leaf.ty {
        GLSLType::Vec2 if matches_fmt(&leaf.hint, "resolution") => Some(vec4(size.x, size.y, 0.0, 0.0)),
        GLSLType::Vec2 if matches_fmt(&leaf.hint, "texel") => Some(vec4(texel.x, texel.y, 0.0, 0.0)),
        // one texel in uv on the longer edge: the finite-difference step a bake wants
        GLSLType::Float if matches_fmt(&leaf.hint, "texel") => Some(Vec4::splat(texel.x.min(texel.y))),
        _ => None,
    }
}

fn put(buf: &mut [u8], at: u32, v: f32) {
    let at = at as usize;
    if at + 4 <= buf.len() { buf[at..at + 4].copy_from_slice(&v.to_le_bytes()); }
}

fn put_i32(buf: &mut [u8], at: u32, v: i32) {
    let at = at as usize;
    if at + 4 <= buf.len() { buf[at..at + 4].copy_from_slice(&v.to_le_bytes()); }
}

/// One block's buffer contents. A member with no value yet writes zeros, which is what a host
/// that forgot to fill it would also get.
pub fn pack_block(block: &Block, values: &UniformValues, size: Vec2) -> Vec<u8> {
    let mut buf = vec![0u8; block.size as usize];

    for op in &block.opaques {
        // a matrix with no control still wants a sane value, and zero is not one
        if !op.type_name.starts_with("mat") { continue; }
        let cols = match op.type_name.as_str() { "mat2" => 2, "mat3" => 3, _ => 4 };
        for c in 0..cols {
            put(&mut buf, op.offset + c * 16 + c * 4, 1.0);
        }
    }

    for leaf in &block.leaves {
        if let Some(v) = bound_to_size(leaf, size) {
            for c in 0..crate::glsl_meta::ty_components(leaf.ty) {
                put(&mut buf, leaf.offset + c as u32 * 4, v[c]);
            }
            continue;
        }
        match leaf.ty {
            GLSLType::Float => if let Some(v) = values.float.get(&leaf.key) { put(&mut buf, leaf.offset, *v); },
            GLSLType::Vec2 => if let Some(v) = values.vec2.get(&leaf.key) {
                put(&mut buf, leaf.offset, v.x);
                put(&mut buf, leaf.offset + 4, v.y);
            },
            GLSLType::Vec3 => if let Some(v) = values.vec3.get(&leaf.key) {
                put(&mut buf, leaf.offset, v.x);
                put(&mut buf, leaf.offset + 4, v.y);
                put(&mut buf, leaf.offset + 8, v.z);
            },
            GLSLType::Vec4 => if let Some(v) = values.vec4.get(&leaf.key) {
                for c in 0..4 { put(&mut buf, leaf.offset + c * 4, v[c as usize]); }
            },
            GLSLType::Int  => if let Some(v) = values.int.get(&leaf.key) { put_i32(&mut buf, leaf.offset, *v); },
            GLSLType::Uint => if let Some(v) = values.uint.get(&leaf.key) { put_i32(&mut buf, leaf.offset, *v as i32); },
            GLSLType::Sampler2D => {}   // not a block member; parse rejects one
        }
    }
    buf
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unified::parse::parse;

    const SRC: &str = "\
#version 450
layout(std140, binding = 0) uniform Globals {
    vec4 weight[2];        // !(default=[0,0,0,0])
    float patch;           // !(default=3)
    vec2 res_px;           // !(format=resolution)
    float pixel_step;      // !(format=texel)
} globals;
#ifdef FRAGMENT
layout(location = 0) out vec4 o_a;   // !(pbr=albedo:rgb)
void main() {}
#endif
";

    fn f32_at(buf: &[u8], at: usize) -> f32 {
        f32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]])
    }

    #[test]
    fn values_land_at_their_std140_offsets() {
        let shader = parse(SRC);
        assert!(shader.errors.is_empty(), "{:?}", shader.errors);
        let mut values = UniformValues::default();
        values.vec4.insert("globals.weight[1]".into(), vec4(1.0, 2.0, 3.0, 4.0));
        values.float.insert("globals.patch".into(), 7.0);

        let block = &shader.blocks[0];
        let buf = pack_block(block, &values, vec2(256.0, 128.0));
        assert_eq!(buf.len(), 64, "two vec4s, a float, a vec2 and a float, padded to 16");
        assert_eq!(f32_at(&buf, 16), 1.0);
        assert_eq!(f32_at(&buf, 28), 4.0);
        assert_eq!(f32_at(&buf, 32), 7.0);
    }

    #[test]
    fn the_render_size_binds_resolution_and_texel_members() {
        let shader = parse(SRC);
        let values = UniformValues::default();
        let buf = pack_block(&shader.blocks[0], &values, vec2(256.0, 128.0));
        // the vec2 is pushed to its own alignment, so it sits at 40 and the last float at 48
        assert_eq!(f32_at(&buf, 40), 256.0);
        assert_eq!(f32_at(&buf, 44), 128.0);
        assert_eq!(f32_at(&buf, 48), 1.0 / 256.0, "one texel on the longer edge");
    }

    #[test]
    fn leaf_specs_carry_the_hints_over_for_the_shared_machinery() {
        let shader = parse(SRC);
        let specs = leaf_specs(&shader);
        let mut values = UniformValues::default();
        values.set_defaults(&specs, vec2(64.0, 64.0));
        assert_eq!(values.float.get("globals.patch"), Some(&3.0));
        assert_eq!(specs.iter().filter(|s| s.name.starts_with("globals.weight")).count(), 2);
    }
}
