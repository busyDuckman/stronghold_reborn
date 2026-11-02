// --------------------------------------------------------------------------------------------------------------------
// The .glsl / .vert pair on disk: watching it, compiling it, and pushing values at it.
//
// A shader is always two files with the same stem. The editor polls both and recompiles on
// either changing, which is the whole point of the tool: save in your editor, see it.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use macroquad::prelude::*;

use crate::glsl_meta::{matches_fmt, parse_uniforms, GLSLType, UniformSpec};
use crate::mq_helpers::alpha_blend_plain;
use crate::tuning::enable_editor_blocks;
use crate::values::UniformValues;

#[derive(Clone, Debug)]
pub struct FilePair { pub frag: PathBuf, pub vert: PathBuf }

#[derive(Clone, Debug)]
pub struct FileStamp { pub frag: SystemTime, pub vert: SystemTime }

pub struct ShaderMaterial { pub mat: Material }

pub fn paired_vert(p: &Path) -> PathBuf {
    let mut v = p.to_path_buf();
    v.set_extension("vert");
    v
}

pub fn stamp_pair(fp: &FilePair) -> std::io::Result<FileStamp> {
    Ok(FileStamp {
        frag: fs::metadata(&fp.frag)?.modified()?,
        vert: fs::metadata(&fp.vert)?.modified()?,
    })
}

pub fn changed(old: &FileStamp, new: &FileStamp) -> bool {
    old.frag != new.frag || old.vert != new.vert
}

// --------------------------------------------------------------------------------------------------------------------
// Compiling.

fn to_uniform_descs(specs: &[UniformSpec]) -> Vec<UniformDesc> {
    let mut v = Vec::with_capacity(specs.len());
    for s in specs {
        let utype = match s.ty {
            GLSLType::Float => UniformType::Float1,
            GLSLType::Vec2  => UniformType::Float2,
            GLSLType::Vec3  => UniformType::Float3,
            GLSLType::Vec4  => UniformType::Float4,
            GLSLType::Int   => UniformType::Int1,
            GLSLType::Uint  => UniformType::Int1,   // macroquad has no uint uniform; Int1 and cast
            GLSLType::Sampler2D => continue,        // samplers go in MaterialParams.textures
        };
        v.push(UniformDesc::new(&s.name, utype));
    }
    v
}

/// Sampler names for MaterialParams.textures, plus the default checkerboard input.
fn to_texture_names(specs: &[UniformSpec]) -> Vec<String> {
    let mut v: Vec<String> = specs.iter()
        .filter(|s| matches!(s.ty, GLSLType::Sampler2D))
        .map(|s| s.name.clone())
        .collect();
    v.push("ShaderTexture".to_string());
    v
}

/// Compile the pair, with the #ifdef EDITOR arms switched on.
pub fn load_material_pair(fp: &FilePair) -> Result<ShaderMaterial, String> {
    let vert_code = fs::read_to_string(&fp.vert).map_err(|e| format!("Read .vert failed: {e}"))?;
    let frag_code = fs::read_to_string(&fp.frag).map_err(|e| format!("Read .glsl failed: {e}"))?;
    let frag_code = enable_editor_blocks(&frag_code);   // the project compiles the raw file instead

    let (specs, _errs) = parse_uniforms(&frag_code);
    let mat = load_material(
        ShaderSource::Glsl { vertex: &vert_code, fragment: &frag_code },
        MaterialParams {
            uniforms: to_uniform_descs(&specs),
            pipeline_params: PipelineParams {
                color_blend: Some(alpha_blend_plain()),
                ..Default::default()
            },
            textures: to_texture_names(&specs),
            ..Default::default()
        },
    ).map_err(|e| format!("{e}"))?;

    Ok(ShaderMaterial { mat })
}

/// Push the current values at the material, once per frame.
/// A format=resolution uniform is bound live from the preview size rather than from the
/// stored value, so resizing the window is reflected without touching anything.
pub fn apply_uniforms(mat: &mut Material, specs: &[UniformSpec], vals: &UniformValues, res_px: Vec2) {
    for s in specs {
        match s.ty {
            GLSLType::Float => if let Some(v) = vals.float.get(&s.name) { mat.set_uniform(&s.name, *v); },
            GLSLType::Vec2 => {
                if matches_fmt(&s.hint, "resolution") {
                    mat.set_uniform(&s.name, res_px);
                } else if let Some(v) = vals.vec2.get(&s.name) {
                    mat.set_uniform(&s.name, *v);
                }
            }
            GLSLType::Vec3 => if let Some(v) = vals.vec3.get(&s.name) { mat.set_uniform(&s.name, *v); },
            GLSLType::Vec4 => if let Some(v) = vals.vec4.get(&s.name) { mat.set_uniform(&s.name, *v); },
            GLSLType::Int  => if let Some(v) = vals.int.get(&s.name) { mat.set_uniform(&s.name, *v); },
            GLSLType::Uint => if let Some(v) = vals.uint.get(&s.name) { mat.set_uniform(&s.name, *v as i32); },
            GLSLType::Sampler2D => {}   // bound from the stand-in map via set_texture
        }
    }
}
