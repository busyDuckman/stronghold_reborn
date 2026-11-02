// --------------------------------------------------------------------------------------------------------------------
// Stand-in textures: what gets bound to a sampler2D uniform while editing.
//
// A shader that samples a texture is useless to preview against nothing, and wiring the
// project's real asset pipeline into a dev tool is how a dev tool becomes a liability. So
// the shader says what it wants in its own default= hint:
//
//   uniform sampler2D u_banner;  // !(default=text:"Crossword", size=[512,128])
//   uniform sampler2D u_tex;     // !(default=path/to/standin.png)
//   uniform sampler2D u_fill;    // !(default=#334455ff, size=[64,64])
//
// text: renders with macroquad's built-in font, so the tool stays clear of the project's
// font stack. Anything unrecognised falls back to a checkerboard, with a warning.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use macroquad::prelude::*;

use crate::glsl_meta::{parse_hex_rgb, parse_hex_rgba, GLSLType, UniformSpec};

#[derive(Clone, Debug, PartialEq)]
pub enum TexSource {
    File(String),
    Text(String),
    Solid(Vec4),
    Checker,
}

#[derive(Clone, Debug)]
pub struct StandIn {
    pub source: TexSource,
    pub tex: Texture2D,
}

pub fn parse_tex_source(s: &str) -> Result<TexSource, String> {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("text:") {
        Ok(TexSource::Text(crate::glsl_meta::unquote(rest).to_string()))
    } else if s.eq_ignore_ascii_case("checker") {
        Ok(TexSource::Checker)
    } else if s.starts_with('#') {
        parse_hex_rgba(s)
            .or_else(|| parse_hex_rgb(s).map(|c| vec4(c.x, c.y, c.z, 1.0)))
            .map(TexSource::Solid)
            .ok_or_else(|| format!("'{}' is not #rrggbb or #rrggbbaa", s))
    } else {
        Ok(TexSource::File(s.to_string()))
    }
}

// --------------------------------------------------------------------------------------------------------------------
// Building the textures.

fn make_solid_tex(w: u32, h: u32, col: Vec4) -> Texture2D {
    let c = Color::new(col.x, col.y, col.z, col.w);
    Texture2D::from_image(&Image::gen_image_color(w as u16, h as u16, c))
}

/// White label on transparent, centred, in macroquad's built-in font.
fn make_text_tex(label: &str, w: u32, h: u32) -> Texture2D {
    let rt = render_target(w, h);
    set_camera(&Camera2D {
        render_target: Some(rt.clone()),
        zoom: vec2(2.0 / w as f32, 2.0 / h as f32),
        target: vec2(w as f32 / 2.0, h as f32 / 2.0),
        ..Default::default()
    });
    clear_background(Color::from_rgba(0, 0, 0, 0));
    let font_size = (h as f32 * 0.6).max(8.0);
    let dims = measure_text(label, None, font_size as u16, 1.0);
    draw_text(label, (w as f32 - dims.width) * 0.5, (h as f32 + dims.offset_y) * 0.5, font_size, WHITE);
    set_default_camera();
    rt.texture.clone()
}

/// Build one stand-in. A missing or undecodable file falls back to the checkerboard, but
/// says so: a silently blank texture reads as a broken shader.
pub fn build_standin_tex(source: &TexSource, w: u32, h: u32, glsl_dir: &Path, name: &str, warns: &mut Vec<String>)
    -> Texture2D
{
    match source {
        TexSource::File(p) => {
            // cwd-relative first (so a repo-root run works), then relative to the .glsl
            let path = if Path::new(p).exists() { PathBuf::from(p) } else { glsl_dir.join(p) };
            match fs::read(&path).ok().and_then(|b| Image::from_file_with_format(&b, None).ok()) {
                Some(img) => Texture2D::from_image(&img),
                None => {
                    warns.push(format!("texture {}: stand-in '{}' not found or not decodable, using checker", name, p));
                    crate::mq_helpers::make_checker_tex(w, h)
                }
            }
        }
        TexSource::Text(label) => make_text_tex(label, w, h),
        TexSource::Solid(col)  => make_solid_tex(w, h, *col),
        TexSource::Checker     => crate::mq_helpers::make_checker_tex(w, h),
    }
}

/// One stand-in per sampler2D uniform. Overrides (from --set) beat the default= hint.
pub fn build_standins(specs: &[UniformSpec], glsl_dir: &Path, overrides: &HashMap<String, TexSource>)
    -> (HashMap<String, StandIn>, Vec<String>)
{
    let mut map = HashMap::new();
    let mut warns = Vec::new();
    for s in specs.iter().filter(|s| matches!(s.ty, GLSLType::Sampler2D)) {
        let (w, h) = s.hint.size.unwrap_or((512, 512));
        let source = match overrides.get(&s.name) {
            Some(src) => src.clone(),
            // a bad default= is already reported by parse_uniforms, so fall back quietly here
            None => s.hint.default.as_deref().and_then(|d| parse_tex_source(d).ok()).unwrap_or(TexSource::Checker),
        };
        let tex = build_standin_tex(&source, w, h, glsl_dir, &s.name, &mut warns);
        map.insert(s.name.clone(), StandIn { source, tex });
    }
    (map, warns)
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_every_source_form() {
        assert_eq!(parse_tex_source(r#"text:"Hi there""#), Ok(TexSource::Text("Hi there".into())));
        assert_eq!(parse_tex_source("checker"), Ok(TexSource::Checker));
        assert_eq!(parse_tex_source("#ff0000"), Ok(TexSource::Solid(vec4(1.0, 0.0, 0.0, 1.0))));
        assert_eq!(parse_tex_source("#ff000080"), Ok(TexSource::Solid(vec4(1.0, 0.0, 0.0, 128.0 / 255.0))));
        assert_eq!(parse_tex_source("art/tile.png"), Ok(TexSource::File("art/tile.png".into())));
    }

    #[test]
    fn a_malformed_colour_is_an_error_not_a_filename() {
        assert!(parse_tex_source("#zzz").is_err());
    }
}
