// --------------------------------------------------------------------------------------------------------------------
// The view pass: what you actually look at.
//
// A unified shader writes a G-buffer, and three buffers of packed numbers tell you nothing.
// So a second pass reads them back through the roles the shader declared and either lights
// them like a material or shows one channel on its own. The pass is WGSL generated at load
// time, because which lane of which target holds roughness is not known until the shader has
// been read; everything the roles do not cover falls back to a sensible constant, so a shader
// that writes four of the twelve channels still previews instead of coming out black.
//
// Tiling matters for a bake: a ground texture that does not join itself is a ground texture
// you will notice in the game, and one tile on its own never shows that. The sampler repeats
// and the view scales the uv, so 2x2 shows the seams.

use std::collections::HashMap;

use crate::unified::roles::{Role, RoleBind, ALL_ROLES};

/// What the preview is showing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewMode {
    Lit,
    Channel(Role),
    Raw(usize),
}

/// The lit view's one light, plus the knobs that make a preview readable.
#[derive(Clone, Copy, Debug)]
pub struct ViewParams {
    pub mode: ViewMode,
    pub azimuth_deg: f32,
    pub elevation_deg: f32,
    pub ambient: f32,
    pub exposure: f32,
    pub tiles: f32,
}

impl Default for ViewParams {
    fn default() -> Self {
        Self { mode: ViewMode::Lit, azimuth_deg: 135.0, elevation_deg: 45.0, ambient: 0.25, exposure: 1.0, tiles: 1.0 }
    }
}

impl ViewParams {
    /// The light direction, pointing from the surface towards the light.
    pub fn light_dir(&self) -> [f32; 3] {
        let az = self.azimuth_deg.to_radians();
        let el = self.elevation_deg.to_radians().clamp(-1.55, 1.55);
        [el.cos() * az.cos(), el.cos() * az.sin(), el.sin().max(0.02)]
    }

    /// The bytes of the View uniform block the generated WGSL reads.
    pub fn bytes(&self) -> Vec<u8> {
        let l = self.light_dir();
        let (mode, role_idx, raw_idx) = match self.mode {
            ViewMode::Lit => (0.0, 0.0, 0.0),
            ViewMode::Channel(r) => (1.0, role_index(r) as f32, 0.0),
            ViewMode::Raw(i) => (2.0, 0.0, i as f32),
        };
        let floats = [
            mode, role_idx, raw_idx, self.tiles.max(0.01),
            l[0], l[1], l[2], self.ambient,
            self.exposure, 0.0, 0.0, 0.0,
        ];
        floats.iter().flat_map(|f| f.to_le_bytes()).collect()
    }
}

pub fn role_index(r: Role) -> usize {
    ALL_ROLES.iter().position(|x| *x == r).unwrap_or(0)
}

const COMPONENT: [&str; 4] = ["x", "y", "z", "w"];

/// `textureSample(tN, samp, uv)` swizzled down to the components a role sits on.
/// One component comes back as a scalar, several as a vector of that many.
fn sample_expr(bind: &RoleBind, want: usize) -> String {
    let sample = format!("textureSample(t{}, samp, uv)", bind.target);
    let lanes: Vec<String> = bind.swizzle.iter().take(want)
        .map(|c| format!("{}.{}", sample, COMPONENT[*c % 4]))
        .collect();
    match lanes.len() {
        0 => "0.0".to_string(),
        1 => lanes[0].clone(),
        n => format!("vec{}<f32>({})", n, lanes.join(", ")),
    }
}

/// One `ch_<role>` accessor: the shader's own lane if it declared one, else the fallback.
/// Only the roles with a fixed component count get one; data is shown by data_view, which
/// builds its own expression because a data lane can be any width.
fn accessor(role: Role, roles: &HashMap<Role, RoleBind>) -> Option<String> {
    let want = role.components()?;
    let ret = if want == 1 { "f32".to_string() } else { format!("vec{}<f32>", want) };
    let body = match roles.get(&role) {
        Some(b) => sample_expr(b, want),
        None => {
            let f = role.fallback();
            if want == 1 { format!("{:?}", f[0]) } else { format!("vec3<f32>({:?}, {:?}, {:?})", f[0], f[1], f[2]) }
        }
    };
    Some(format!("fn ch_{}(uv: vec2<f32>) -> {} {{ return {}; }}", role.name(), ret, body))
}

/// The channel-isolate switch: one arm per role, whether or not the shader declares it, so the
/// view buttons and the shader arms cannot drift apart.
fn role_switch(roles: &HashMap<Role, RoleBind>) -> String {
    let mut arms = String::new();
    for (i, role) in ALL_ROLES.iter().enumerate() {
        let body = match role {
            // a normal map is looked at packed, which is how it is stored and how it is judged
            Role::Normal => "ch_normal(uv)".to_string(),
            Role::NormalSigned => "ch_normal_signed(uv) * 0.5 + vec3<f32>(0.5)".to_string(),
            Role::Data => data_view(roles),
            r if r.components() == Some(1) => format!("vec3<f32>(ch_{}(uv))", r.name()),
            r => format!("ch_{}(uv)", r.name()),
        };
        arms.push_str(&format!("        case {}: {{ return {}; }}\n", i, body));
    }
    format!("fn role_view(uv: vec2<f32>, idx: i32) -> vec3<f32> {{\n    switch (idx) {{\n{}        default: {{ return vec3<f32>(1.0, 0.0, 1.0); }}\n    }}\n}}", arms)
}

/// The data role carries no meaning, so it is shown as whatever lanes it has, padded to a colour.
fn data_view(roles: &HashMap<Role, RoleBind>) -> String {
    let Some(b) = roles.get(&Role::Data) else { return "vec3<f32>(0.0)".to_string() };
    match b.swizzle.len() {
        1 => format!("vec3<f32>({})", sample_expr(b, 1)),
        2 => format!("vec3<f32>({}, 0.0)", sample_expr(b, 2)),
        _ => sample_expr(b, 3),
    }
}

fn raw_switch(n_targets: usize) -> String {
    let mut arms = String::new();
    for i in 0..n_targets {
        arms.push_str(&format!("        case {}: {{ return textureSample(t{}, samp, uv); }}\n", i, i));
    }
    format!("fn raw_view(uv: vec2<f32>, idx: i32) -> vec4<f32> {{\n    switch (idx) {{\n{}        default: {{ return vec4<f32>(1.0, 0.0, 1.0, 1.0); }}\n    }}\n}}", arms)
}

/// The whole view pass, for this shader's roles and target count.
pub fn generate(roles: &HashMap<Role, RoleBind>, n_targets: usize) -> String {
    let mut src = String::new();
    src.push_str("// Generated by shader_editor for one shader's declared PBR roles. Not a file on disk.\n");
    src.push_str("struct View {\n    mode: vec4<f32>,\n    light: vec4<f32>,\n    knobs: vec4<f32>,\n}\n\n");
    src.push_str("@group(0) @binding(0) var samp: sampler;\n");
    src.push_str("@group(0) @binding(1) var<uniform> view: View;\n");
    for i in 0..n_targets {
        src.push_str(&format!("@group(0) @binding({}) var t{}: texture_2d<f32>;\n", i + 2, i));
    }
    src.push('\n');

    for role in ALL_ROLES {
        if let Some(fn_src) = accessor(role, roles) {
            src.push_str(&fn_src);
            src.push('\n');
        }
    }
    src.push('\n');

    src.push_str(NORMAL_AND_LIT);
    src.push('\n');
    src.push_str(&role_switch(roles));
    src.push_str("\n\n");
    src.push_str(&raw_switch(n_targets.max(1)));
    src.push_str("\n\n");
    src.push_str(VERTEX_AND_FRAGMENT);
    src
}

/// The shading itself: one directional light, Lambert plus a GGX highlight, ambient scaled by
/// occlusion. Deliberately small. It is here to tell you whether your normal map has the right
/// sign and whether your roughness lane is in the range you think, not to match a renderer.
const NORMAL_AND_LIT: &str = r#"
fn shading_normal(uv: vec2<f32>) -> vec3<f32> {
    let from_packed = ch_normal(uv) * 2.0 - vec3<f32>(1.0);
    let from_signed = ch_normal_signed(uv);
    let n = select(from_packed, from_signed, HAS_SIGNED_NORMAL);
    return normalize(select(vec3<f32>(0.0, 0.0, 1.0), n, length(n) > 1e-4));
}

fn lit(uv: vec2<f32>) -> vec3<f32> {
    let n = shading_normal(uv);
    let wet = clamp(ch_wetness(uv), 0.0, 1.0);
    // wet ground is darker and smoother: the reason a shader bothers to write a wetness lane
    let albedo = ch_albedo(uv) * mix(1.0, 0.55, wet);
    let rough = clamp(mix(clamp(ch_roughness(uv), 0.04, 1.0), 0.08, wet), 0.04, 1.0);
    let metal = clamp(ch_metallic(uv), 0.0, 1.0);
    let ao = clamp(ch_ao(uv), 0.0, 1.0);

    let l = normalize(view.light.xyz);
    let v = vec3<f32>(0.0, 0.0, 1.0);
    let h = normalize(l + v);
    let ndl = max(dot(n, l), 0.0);
    let ndh = max(dot(n, h), 0.0);
    let ndv = max(dot(n, v), 1e-4);

    let a = rough * rough;
    let a2 = a * a;
    let d = ndh * ndh * (a2 - 1.0) + 1.0;
    let ggx = a2 / max(3.14159265 * d * d, 1e-6);
    let k = a * 0.5;
    let gv = ndv / (ndv * (1.0 - k) + k);
    let gl = ndl / (ndl * (1.0 - k) + k);
    let f0 = mix(vec3<f32>(0.04), albedo, metal);
    let fres = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - ndh, 5.0);

    let spec = fres * (ggx * gv * gl / max(4.0 * ndv * ndl, 1e-4)) * ndl * ch_specular_gate(uv);
    let diffuse = albedo * (1.0 - metal) * ndl;
    let ambient = albedo * view.light.w * ao;
    return (diffuse + spec + ambient) * view.knobs.x + ch_emissive(uv);
}
"#;

/// The specular lane is optional, and a shader that does not write one should not lose its
/// highlight, so an absent specular reads as 1 rather than as the role's 0 fallback.
const SPECULAR_GATE_PRESENT: &str = "fn ch_specular_gate(uv: vec2<f32>) -> f32 { return ch_specular(uv); }\n";
const SPECULAR_GATE_ABSENT: &str = "fn ch_specular_gate(uv: vec2<f32>) -> f32 { return 1.0; }\n";

const VERTEX_AND_FRAGMENT: &str = r#"
struct VsOut {
    @builtin(position) pos: vec4<f32>,
}

// A fullscreen triangle. It carries no uv of its own on purpose: the fragment works out which
// texel it wants from its own pixel position, so this pass cannot introduce a flip whatever
// convention the shader's vertex stage used. Target pixel (0,0) reads G-buffer texel (0,0),
// and the readback keeps that order, so the preview, the PNG and the texture agree.
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let c = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: VsOut;
    o.pos = vec4<f32>(c.x * 2.0 - 1.0, 1.0 - c.y * 2.0, 0.0, 1.0);
    return o;
}

@fragment
fn fs_main(frag: VsOut) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(t0, 0));
    let uv = (frag.pos.xy / dims) * view.mode.w;
    let mode = i32(view.mode.x);
    var c: vec3<f32>;
    if (mode == 0) {
        c = lit(uv);
    } else if (mode == 1) {
        c = role_view(uv, i32(view.mode.y));
    } else {
        c = raw_view(uv, i32(view.mode.z)).rgb;
    }
    return vec4<f32>(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
"#;

/// Fill in the two compile-time constants the shading code reads, now that the roles are known.
pub fn generate_final(roles: &HashMap<Role, RoleBind>, n_targets: usize) -> String {
    let body = generate(roles, n_targets);
    let signed = roles.contains_key(&Role::NormalSigned);
    let gate = if roles.contains_key(&Role::Specular) { SPECULAR_GATE_PRESENT } else { SPECULAR_GATE_ABSENT };
    format!("const HAS_SIGNED_NORMAL: bool = {};\n{}\n{}", signed, gate, body)
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unified::roles::resolve;

    fn terrain_roles() -> HashMap<Role, RoleBind> {
        let binds = vec![
            RoleBind::new(Role::Albedo, 0, vec![0, 1, 2]),
            RoleBind::new(Role::Normal, 1, vec![0, 1, 2]),
            RoleBind::new(Role::Height, 1, vec![3]),
            RoleBind::new(Role::Roughness, 2, vec![0]),
            RoleBind::new(Role::Wetness, 2, vec![1]),
            RoleBind::new(Role::Data, 2, vec![2]),
        ];
        resolve(&binds).0
    }

    /// The generated pass has to be valid WGSL, and naga will say so without a GPU in sight.
    fn assert_wgsl_ok(src: &str) {
        let module = match wgpu::naga::front::wgsl::parse_str(src) {
            Ok(m) => m,
            Err(e) => panic!("generated WGSL does not parse: {}\n{}", e.emit_to_string(src), numbered(src)),
        };
        let mut v = wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        );
        if let Err(e) = v.validate(&module) {
            panic!("generated WGSL does not validate: {:?}\n{}", e, numbered(src));
        }
    }

    fn numbered(src: &str) -> String {
        src.lines().enumerate().map(|(i, l)| format!("{:4} {}\n", i + 1, l)).collect()
    }

    #[test]
    fn the_generated_pass_is_valid_wgsl_for_a_full_g_buffer() {
        assert_wgsl_ok(&generate_final(&terrain_roles(), 3));
    }

    #[test]
    fn it_is_valid_for_a_shader_that_declares_almost_nothing() {
        let binds = vec![RoleBind::new(Role::Albedo, 0, vec![0, 1, 2])];
        assert_wgsl_ok(&generate_final(&resolve(&binds).0, 1));
    }

    #[test]
    fn it_is_valid_for_signed_normals_and_a_two_lane_data_target() {
        let binds = vec![
            RoleBind::new(Role::NormalSigned, 0, vec![0, 1, 2]),
            RoleBind::new(Role::Specular, 1, vec![0]),
            RoleBind::new(Role::Data, 1, vec![1, 2]),
        ];
        assert_wgsl_ok(&generate_final(&resolve(&binds).0, 2));
    }

    #[test]
    fn a_declared_lane_is_sampled_and_an_absent_one_falls_back() {
        let src = generate_final(&terrain_roles(), 3);
        assert!(src.contains("fn ch_roughness(uv: vec2<f32>) -> f32 { return textureSample(t2, samp, uv).x; }"), "{src}");
        assert!(src.contains("fn ch_height(uv: vec2<f32>) -> f32 { return textureSample(t1, samp, uv).w; }"), "{src}");
        assert!(src.contains("fn ch_metallic(uv: vec2<f32>) -> f32 { return 0.0; }"), "{src}");
        assert!(src.contains("const HAS_SIGNED_NORMAL: bool = false;"), "{src}");
    }

    #[test]
    fn the_view_uniform_is_twelve_floats_in_the_order_the_wgsl_reads_them() {
        let p = ViewParams { mode: ViewMode::Raw(2), tiles: 4.0, ..Default::default() };
        let b = p.bytes();
        assert_eq!(b.len(), 48);
        assert_eq!(f32::from_le_bytes([b[0], b[1], b[2], b[3]]), 2.0, "mode");
        assert_eq!(f32::from_le_bytes([b[8], b[9], b[10], b[11]]), 2.0, "raw index");
        assert_eq!(f32::from_le_bytes([b[12], b[13], b[14], b[15]]), 4.0, "tiles");
    }

    #[test]
    fn the_light_never_points_straight_at_the_surface_from_behind() {
        let p = ViewParams { elevation_deg: -90.0, ..Default::default() };
        assert!(p.light_dir()[2] > 0.0, "a light below the plane would black the preview out");
    }
}
