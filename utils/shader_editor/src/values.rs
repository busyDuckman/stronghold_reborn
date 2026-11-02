// --------------------------------------------------------------------------------------------------------------------
// The live value of every uniform, keyed by name and split by GLSL type.
//
// Kept apart from the specs so a reload can reparse the shader without losing what the
// sliders are currently set to. Samplers are absent on purpose: a texture lives in the
// stand-in map, not here.

use std::collections::HashMap;

use macroquad::prelude::*;

use crate::glsl_meta::{
    matches_fmt, parse_hex_rgb, parse_hex_rgba, parse_vec2_str, parse_vec3_str, parse_vec4_str,
    GLSLType, UniformSpec,
};

#[derive(Clone, Debug, Default)]
pub struct UniformValues {
    pub float: HashMap<String, f32>,
    pub vec2:  HashMap<String, Vec2>,
    pub vec3:  HashMap<String, Vec3>,
    pub vec4:  HashMap<String, Vec4>,
    pub int:   HashMap<String, i32>,
    pub uint:  HashMap<String, u32>,
}

impl UniformValues {
    /// Fill in anything not already set, from each uniform's default= hint.
    /// or_insert throughout, so a saved tuning const and a live tweak both survive a reload.
    pub fn set_defaults(&mut self, specs: &[UniformSpec], tex_size: Vec2) {
        for s in specs {
            match s.ty {
                GLSLType::Float => {
                    let v = s.hint.default.as_deref().and_then(|d| d.parse::<f32>().ok()).unwrap_or(0.0);
                    self.float.entry(s.name.clone()).or_insert(v);
                }
                GLSLType::Vec2 => {
                    let v = if matches_fmt(&s.hint, "resolution") {
                        tex_size
                    } else {
                        s.hint.default.as_deref().and_then(parse_vec2_str).unwrap_or(Vec2::ZERO)
                    };
                    self.vec2.entry(s.name.clone()).or_insert(v);
                }
                GLSLType::Vec3 => {
                    let parse = if matches_fmt(&s.hint, "RGB") { parse_hex_rgb } else { parse_vec3_str };
                    let v = s.hint.default.as_deref().and_then(parse).unwrap_or(Vec3::ONE);
                    self.vec3.entry(s.name.clone()).or_insert(v);
                }
                GLSLType::Vec4 => {
                    let parse = if matches_fmt(&s.hint, "RGBA") { parse_hex_rgba } else { parse_vec4_str };
                    let v = s.hint.default.as_deref().and_then(parse).unwrap_or(Vec4::ONE);
                    self.vec4.entry(s.name.clone()).or_insert(v);
                }
                GLSLType::Int => {
                    let v = s.hint.default.as_deref().and_then(|d| d.parse::<i32>().ok()).unwrap_or(0);
                    self.int.entry(s.name.clone()).or_insert(v);
                }
                GLSLType::Uint => {
                    let v = s.hint.default.as_deref().and_then(|d| d.parse::<u32>().ok()).unwrap_or(0);
                    self.uint.entry(s.name.clone()).or_insert(v);
                }
                GLSLType::Sampler2D => {}   // textures live in the stand-in map
            }
        }
    }
}

/// Apply one --set override, parsed against the uniform's declared type.
/// Samplers never reach here; the caller routes those to the texture-override map.
pub fn apply_set_override(values: &mut UniformValues, spec: &UniformSpec, raw: &str) -> Result<(), String> {
    let bad = |what: &str| format!("--set {}: '{}' is not {}", spec.name, raw, what);
    match spec.ty {
        GLSLType::Float => {
            let v = raw.parse::<f32>().map_err(|_| bad("a float"))?;
            values.float.insert(spec.name.clone(), v);
        }
        GLSLType::Vec2 => {
            let v = parse_vec2_str(raw).ok_or_else(|| bad("a vec2"))?;
            values.vec2.insert(spec.name.clone(), v);
        }
        GLSLType::Vec3 => {
            let v = if raw.starts_with('#') { parse_hex_rgb(raw) } else { parse_vec3_str(raw) };
            values.vec3.insert(spec.name.clone(), v.ok_or_else(|| bad("a vec3 or #rrggbb"))?);
        }
        GLSLType::Vec4 => {
            let v = if raw.starts_with('#') { parse_hex_rgba(raw) } else { parse_vec4_str(raw) };
            values.vec4.insert(spec.name.clone(), v.ok_or_else(|| bad("a vec4 or #rrggbbaa"))?);
        }
        GLSLType::Int => {
            let v = raw.parse::<i32>().map_err(|_| bad("an int"))?;
            values.int.insert(spec.name.clone(), v);
        }
        GLSLType::Uint => {
            let v = raw.parse::<u32>().map_err(|_| bad("a uint"))?;
            values.uint.insert(spec.name.clone(), v);
        }
        GLSLType::Sampler2D => return Err(format!("--set {}: sampler overrides are handled separately", spec.name)),
    }
    Ok(())
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glsl_meta::parse_uniforms;

    fn specs_of(src: &str) -> Vec<UniformSpec> {
        parse_uniforms(src).0
    }

    #[test]
    fn defaults_come_from_the_hints() {
        let specs = specs_of("uniform float u_a; // !(default=2.5)\nuniform vec3 u_c; // !(format=RGB, default=#ff0000)\n");
        let mut values = UniformValues::default();
        values.set_defaults(&specs, vec2(64.0, 32.0));
        assert_eq!(values.float.get("u_a"), Some(&2.5));
        assert_eq!(values.vec3.get("u_c"), Some(&vec3(1.0, 0.0, 0.0)));
    }

    #[test]
    fn a_resolution_uniform_takes_the_preview_size() {
        let specs = specs_of("uniform vec2 u_res; // !(format=resolution, default=[8,8])\n");
        let mut values = UniformValues::default();
        values.set_defaults(&specs, vec2(640.0, 360.0));
        assert_eq!(values.vec2.get("u_res"), Some(&vec2(640.0, 360.0)));
    }

    #[test]
    fn an_existing_value_survives_a_reseed() {
        let specs = specs_of("uniform float u_a; // !(default=2.5)\n");
        let mut values = UniformValues::default();
        values.float.insert("u_a".into(), 9.0);
        values.set_defaults(&specs, Vec2::ONE);
        assert_eq!(values.float.get("u_a"), Some(&9.0), "a live tweak must survive a reload");
    }

    #[test]
    fn set_override_is_type_checked() {
        let specs = specs_of("uniform float u_a;\n");
        let mut values = UniformValues::default();
        assert!(apply_set_override(&mut values, &specs[0], "not_a_number").is_err());
        assert!(apply_set_override(&mut values, &specs[0], "0.75").is_ok());
        assert_eq!(values.float.get("u_a"), Some(&0.75));
    }
}
