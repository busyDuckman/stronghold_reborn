// --------------------------------------------------------------------------------------------------------------------
// BasicRenderer3d: a terrain mesh with the game's sprites standing on it as billboards.
//
// The mesh comes from either the high resolution DEM (one vertex a sample) or the game map
// cells (two flat triangles a cell on the game's own diagonal), coloured by ground type and
// lit by a fixed sun. Trees, buildings and units are camera-facing quads cut from the
// sprite atlas, alpha tested so they need no sorting. Distance fog fades everything into
// the sky colour. When a skybox hands over an image it is blitted behind everything with its
// bottom on the horizon and its aspect kept, black showing wherever it does not reach. The
// geometry builders at the top are plain CPU code and are tested; the GPU half below them is
// exercised by running the app.

use crate::camera::Camera;
use crate::game_map::{Decor, GameMap, GroundType, Triangle, TERRAIN_DIAGONAL};
use crate::gpu::{Target, TARGET_FORMAT};
use crate::renderer::{RenderContext, RendererUiOutcome, ViewportRenderer};
use crate::skybox::{SkyImage, Skybox};
use crate::sprite_atlas::{AtlasSprite, SpriteAtlas, SpriteKey};
use crate::top_down::{ground_colour, side_colour};
use crate::view_options::ViewOptions;
use crate::world::World;
use anyhow::{anyhow, Result};
use bytemuck::{Pod, Zeroable};
use std::sync::Arc;
use wgpu::util::DeviceExt;

/// Which mesh the terrain is built from.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum MeshSource {
    Dem,
    Cells,
}

/// The renderer's own knobs.
#[derive(Clone, PartialEq, Debug)]
pub struct Basic3dOptions {
    pub mesh: MeshSource,
    /// Cells per sprite pixel.
    pub sprite_scale: f32,
    pub fog: bool,
    pub show_trees: bool,
    pub show_buildings: bool,
    pub show_units: bool,
}

impl Default for Basic3dOptions {
    fn default() -> Basic3dOptions {
        Basic3dOptions { mesh: MeshSource::Dem, sprite_scale: 0.025, fog: true, show_trees: true, show_buildings: true, show_units: true }
    }
}

/// A terrain vertex: position in world units and a lit colour.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Debug, PartialEq)]
pub struct TerrainVertex {
    pub pos: [f32; 3],
    pub colour: [f32; 4],
}

/// A billboard vertex: position, atlas coordinates and a tint (white for real art).
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Debug, PartialEq)]
pub struct SpriteVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub tint: [f32; 4],
}

/// A sky vertex: a clip-space corner and where it samples the sky image.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Debug, PartialEq)]
pub struct SkyVertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
}

/// An indexed triangle list.
#[derive(Default)]
pub struct TerrainMesh {
    pub vertices: Vec<TerrainVertex>,
    pub indices: Vec<u32>,
}

/// The sun, pointing from the ground towards the light.
const SUN: [f32; 3] = [-0.45, -0.35, 0.82];
const SKY: [f32; 4] = [0.60, 0.69, 0.82, 1.0];
/// The far plane in cells, and where fog starts and finishes.
const FAR: f32 = 400.0;
const FOG_START: f32 = 60.0;
const FOG_END: f32 = 320.0;

// --------------------------------------------------------------------------------------------------------------------
// Geometry

fn normalise(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
    [v[0] / len, v[1] / len, v[2] / len]
}

/// How lit a surface with this normal is, 0.4 in shadow to 1.0 facing the sun.
fn lit(normal: [f32; 3]) -> f32 {
    let sun = normalise(SUN);
    let n = normalise(normal);
    let facing = (n[0] * sun[0] + n[1] * sun[1] + n[2] * sun[2]).max(0.0);
    0.4 + 0.6 * facing
}

fn shade(colour: [f32; 4], factor: f32) -> [f32; 4] {
    [colour[0] * factor, colour[1] * factor, colour[2] * factor, colour[3]]
}

/// The ground type under a point inside a cell, picking the triangle the point falls in.
fn ground_under(map: &GameMap, x: i32, y: i32, fx: f32, fy: f32) -> GroundType {
    let Some(ground) = map.ground_at(x, y) else { return GroundType::Water };
    let rising = map.splits_rising(x, y).unwrap_or(false);
    // A rising split runs from top-right to bottom-left; A holds the bottom-left corner.
    let in_a = if rising { fx + fy <= 1.0 } else { fy >= fx };
    ground[if in_a { Triangle::A } else { Triangle::B } as usize]
}

/// One vertex a DEM sample, lit from the neighbouring samples' slope.
pub fn build_dem_mesh(world: &World, height_scale: f32) -> TerrainMesh {
    let dem = world.dem();
    let map = world.map();
    let (cols, rows, factor) = (dem.cols(), dem.rows(), dem.factor() as f32);
    let sample = |i: usize, j: usize| dem.sample(i.min(cols - 1), j.min(rows - 1)).unwrap_or(0.0);
    let mut vertices = Vec::with_capacity(cols * rows);
    for j in 0..rows {
        for i in 0..cols {
            let h = sample(i, j);
            let (u, v) = (i as f32 / factor, j as f32 / factor);
            let ground = ground_under(map, u.floor() as i32, v.floor() as i32, u.fract(), v.fract());
            let dx = (sample(i + 1, j) - sample(i.saturating_sub(1), j)) * height_scale * factor * 0.5;
            let dy = (sample(i, j + 1) - sample(i, j.saturating_sub(1))) * height_scale * factor * 0.5;
            let normal = [-dx, -dy, 1.0];
            let light = if ground == GroundType::Water { 0.9 } else { lit(normal) };
            vertices.push(TerrainVertex { pos: [u, v, h * height_scale], colour: shade(ground_colour(ground), light) });
        }
    }
    let mut indices = Vec::with_capacity((cols - 1) * (rows - 1) * 6);
    for j in 0..rows - 1 {
        for i in 0..cols - 1 {
            let a = (j * cols + i) as u32;
            let b = a + 1;
            let c = a + cols as u32;
            let d = c + 1;
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    TerrainMesh { vertices, indices }
}

/// Two flat triangles a cell, split the way the game splits them, each its own colour.
pub fn build_cell_mesh(world: &World, height_scale: f32) -> TerrainMesh {
    let map = world.map();
    let mut mesh = TerrainMesh::default();
    for y in 0..map.rows() as i32 {
        for x in 0..map.cols() as i32 {
            let Some(ground) = map.ground_at(x, y) else { continue };
            let Some(word) = map.terrain_at(x, y) else { continue };
            let rising = word & TERRAIN_DIAGONAL != 0;
            let [h00, h10, h01, h11] = map.corner_heights(x, y).map(|h| h as f32 * height_scale);
            let (fx, fy) = (x as f32, y as f32);
            let tl = [fx, fy, h00];
            let tr = [fx + 1.0, fy, h10];
            let bl = [fx, fy + 1.0, h01];
            let br = [fx + 1.0, fy + 1.0, h11];
            let (tri_a, tri_b) = if rising { ([tl, bl, tr], [bl, br, tr]) } else { ([tl, bl, br], [tl, br, tr]) };
            for (tri, kind) in [(tri_a, ground[0]), (tri_b, ground[1])] {
                let e1 = [tri[1][0] - tri[0][0], tri[1][1] - tri[0][1], tri[1][2] - tri[0][2]];
                let e2 = [tri[2][0] - tri[0][0], tri[2][1] - tri[0][1], tri[2][2] - tri[0][2]];
                let normal = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
                let normal = if normal[2] < 0.0 { [-normal[0], -normal[1], -normal[2]] } else { normal };
                let light = if kind == GroundType::Water { 0.9 } else { lit(normal) };
                let colour = shade(ground_colour(kind), light);
                let base = mesh.vertices.len() as u32;
                for p in tri {
                    mesh.vertices.push(TerrainVertex { pos: p, colour });
                }
                mesh.indices.extend_from_slice(&[base, base + 1, base + 2]);
            }
        }
    }
    mesh
}

/// What a billboard shows and how big it is when there is no atlas to ask.
struct Billboard {
    key: SpriteKey,
    foot: [f32; 3],
    tint: [f32; 4],
    fallback_size: (f32, f32),
}

/// Camera-facing quads for everything standing on the map, six vertices each.
/// The height scale must be the one the terrain mesh was built with, or the feet leave the
/// ground.
pub fn build_billboards(world: &World, camera: &Camera, height_scale: f32, knobs: &Basic3dOptions, atlas: Option<&SpriteAtlas>) -> Vec<SpriteVertex> {
    let map = world.map();
    let scale = height_scale;
    let mut boards: Vec<Billboard> = Vec::new();
    for y in 0..map.rows() as i32 {
        for x in 0..map.cols() as i32 {
            let foot = [x as f32 + 0.5, y as f32 + 0.5, map.height_bilinear(x as f32 + 0.5, y as f32 + 0.5) * scale];
            match map.decor_at(x, y) {
                Some(Decor::Tree { kind }) if knobs.show_trees => {
                    boards.push(Billboard { key: SpriteKey::Tree { kind }, foot, tint: [0.1, 0.4, 0.1, 1.0], fallback_size: (48.0, 55.0) });
                }
                Some(Decor::Building { kind, upgrade, .. }) if knobs.show_buildings => {
                    let key = SpriteKey::Building { kind, upgrade: upgrade.clamp(1, 3), view: 1 };
                    boards.push(Billboard { key, foot, tint: [0.55, 0.4, 0.25, 1.0], fallback_size: (100.0, 65.0) });
                }
                _ => {}
            }
        }
    }
    if knobs.show_units {
        for sprite in world.sprites() {
            let foot = [sprite.x + 0.5, sprite.y + 0.5, map.height_bilinear(sprite.x + 0.5, sprite.y + 0.5) * scale];
            boards.push(Billboard { key: SpriteKey::Unit { kind: sprite.kind }, foot, tint: side_colour(sprite.side), fallback_size: (20.0, 21.0) });
        }
    }
    let [rx, ry] = camera.right();
    let mut vertices = Vec::with_capacity(boards.len() * 6);
    for board in boards {
        let (left, right, height, uv, tint) = match atlas.and_then(|a| a.lookup(board.key)) {
            Some(sprite) => {
                let (w, h) = (sprite.width as f32, sprite.height as f32);
                // Units carry their foot in the anchor; trees and buildings stand bottom-centre.
                let left = if sprite.kind == 4 { sprite.anchor_x as f32 } else { w * 0.5 };
                (left, w - left, h, sprite.uv, [1.0, 1.0, 1.0, 1.0])
            }
            None => (board.fallback_size.0 * 0.5, board.fallback_size.0 * 0.5, board.fallback_size.1, [0.0, 0.0, 0.0, 0.0], board.tint),
        };
        let s = knobs.sprite_scale;
        let (l, r, top) = (left * s, right * s, height * s);
        let f = board.foot;
        let bl = [f[0] - rx * l, f[1] - ry * l, f[2]];
        let br = [f[0] + rx * r, f[1] + ry * r, f[2]];
        let tl = [bl[0], bl[1], f[2] + top];
        let tr = [br[0], br[1], f[2] + top];
        let [u0, v0, u1, v1] = uv;
        let corner = |pos: [f32; 3], u: f32, v: f32| SpriteVertex { pos, uv: [u, v], tint };
        vertices.extend_from_slice(&[
            corner(tl, u0, v0),
            corner(bl, u0, v1),
            corner(br, u1, v1),
            corner(tl, u0, v0),
            corner(br, u1, v1),
            corner(tr, u1, v0),
        ]);
    }
    vertices
}

/// The sky quad in clip space: full width, bottom on the horizon (the middle row, as the
/// camera has no pitch), as tall as the image is at that width so nothing is stretched.
pub fn sky_quad(image_w: u32, image_h: u32, target_w: u32, target_h: u32) -> [SkyVertex; 6] {
    let on_screen_h = target_w as f32 * image_h as f32 / image_w.max(1) as f32;
    let top = 2.0 * on_screen_h / target_h.max(1) as f32;
    let (bl, br) = (SkyVertex { pos: [-1.0, 0.0], uv: [0.0, 1.0] }, SkyVertex { pos: [1.0, 0.0], uv: [1.0, 1.0] });
    let (tl, tr) = (SkyVertex { pos: [-1.0, top], uv: [0.0, 0.0] }, SkyVertex { pos: [1.0, top], uv: [1.0, 0.0] });
    [bl, br, tr, bl, tr, tl]
}

// --------------------------------------------------------------------------------------------------------------------
// The GPU side

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Uniforms {
    view_proj: [f32; 16],
    fog_colour: [f32; 4],
    /// fog start, fog end, unused, unused
    fog: [f32; 4],
    eye: [f32; 4],
}

const SHADER: &str = r#"
struct Uniforms {
    view_proj: mat4x4<f32>,
    fog_colour: vec4<f32>,
    fog: vec4<f32>,
    eye: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(1) @binding(0) var atlas_texture: texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler: sampler;

fn fogged(colour: vec4<f32>, dist: f32) -> vec4<f32> {
    let f = clamp((dist - u.fog.x) / max(u.fog.y - u.fog.x, 0.001), 0.0, 1.0);
    return mix(colour, u.fog_colour, f);
}

struct TerrainIn {
    @location(0) pos: vec3<f32>,
    @location(1) colour: vec4<f32>,
};
struct TerrainOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) colour: vec4<f32>,
    @location(1) dist: f32,
};

@vertex
fn vs_terrain(in: TerrainIn) -> TerrainOut {
    var out: TerrainOut;
    out.clip = u.view_proj * vec4<f32>(in.pos, 1.0);
    out.colour = in.colour;
    out.dist = distance(in.pos, u.eye.xyz);
    return out;
}

@fragment
fn fs_terrain(in: TerrainOut) -> @location(0) vec4<f32> {
    return fogged(in.colour, in.dist);
}

struct SpriteIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) tint: vec4<f32>,
};
struct SpriteOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec4<f32>,
    @location(2) dist: f32,
};

@vertex
fn vs_sprite(in: SpriteIn) -> SpriteOut {
    var out: SpriteOut;
    out.clip = u.view_proj * vec4<f32>(in.pos, 1.0);
    out.uv = in.uv;
    out.tint = in.tint;
    out.dist = distance(in.pos, u.eye.xyz);
    return out;
}

@fragment
fn fs_sprite(in: SpriteOut) -> @location(0) vec4<f32> {
    let texel = textureSample(atlas_texture, atlas_sampler, in.uv) * in.tint;
    if (texel.a < 0.5) {
        discard;
    }
    return fogged(vec4<f32>(texel.rgb, 1.0), in.dist);
}

struct SkyIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
};
struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_sky(in: SkyIn) -> SkyOut {
    var out: SkyOut;
    out.clip = vec4<f32>(in.pos, 0.0, 1.0);
    out.uv = in.uv;
    return out;
}

@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSample(atlas_texture, atlas_sampler, in.uv).rgb, 1.0);
}
"#;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// A vertex buffer that grows when it must.
struct GrowingBuffer {
    buffer: Option<wgpu::Buffer>,
    capacity: usize,
    label: &'static str,
    usage: wgpu::BufferUsages,
}

impl GrowingBuffer {
    fn new(label: &'static str, usage: wgpu::BufferUsages) -> GrowingBuffer {
        GrowingBuffer { buffer: None, capacity: 0, label, usage }
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, bytes: &[u8]) {
        if bytes.len() > self.capacity || self.buffer.is_none() {
            let capacity = bytes.len().max(4096).next_power_of_two();
            self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(self.label),
                size: capacity as u64,
                usage: self.usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
            self.capacity = capacity;
        }
        if let (Some(buffer), false) = (&self.buffer, bytes.is_empty()) {
            queue.write_buffer(buffer, 0, bytes);
        }
    }
}

pub struct BasicRenderer3d {
    knobs: Basic3dOptions,
    terrain_pipeline: wgpu::RenderPipeline,
    sprite_pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    uniform_bind: wgpu::BindGroup,
    atlas_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    atlas_bind: wgpu::BindGroup,
    /// The atlas the billboards are cut from, or None for the flat colour stand-ins.
    billboard_atlas: Option<Arc<SpriteAtlas>>,
    terrain_vertices: GrowingBuffer,
    terrain_indices: GrowingBuffer,
    index_count: u32,
    /// The height scale the terrain on the GPU was built with. The sprites and the eye use this
    /// one, not the live option, so nothing can leave the ground between a change and a rebuild.
    height_scale: f32,
    sprite_vertices: GrowingBuffer,
    depth: Option<(wgpu::TextureView, u32, u32)>,
    sky_pipeline: wgpu::RenderPipeline,
    sky_vertices: GrowingBuffer,
    /// The sky image on the GPU, remade when its size changes and rewritten every frame.
    sky: Option<SkyTexture>,
}

struct SkyTexture {
    texture: wgpu::Texture,
    bind: wgpu::BindGroup,
    width: u32,
    height: u32,
}

impl BasicRenderer3d {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> BasicRenderer3d {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("basic_3d"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("basic_3d_uniforms"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("basic_3d_atlas"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("basic_3d_uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("basic_3d_uniforms"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("basic_3d"),
            bind_group_layouts: &[&uniform_layout, &atlas_layout],
            push_constant_ranges: &[],
        });
        let depth_state = wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        };
        // The sky quad neither reads nor writes depth: it is drawn first and everything covers it.
        let sky_depth_state = wgpu::DepthStencilState { depth_write_enabled: false, depth_compare: wgpu::CompareFunction::Always, ..depth_state.clone() };
        let make_pipeline = |label: &str, vs: &str, fs: &str, buffers: &[wgpu::VertexBufferLayout], depth: &wgpu::DepthStencilState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some(vs), buffers, compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState { format: TARGET_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: Some(depth.clone()),
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            })
        };
        let terrain_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<TerrainVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4],
        };
        let sprite_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<SpriteVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x4],
        };
        let sky_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<SkyVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2],
        };
        let terrain_pipeline = make_pipeline("basic_3d_terrain", "vs_terrain", "fs_terrain", &[terrain_layout], &depth_state);
        let sprite_pipeline = make_pipeline("basic_3d_sprites", "vs_sprite", "fs_sprite", &[sprite_layout], &depth_state);
        let sky_pipeline = make_pipeline("basic_3d_sky", "vs_sky", "fs_sky", &[sky_layout], &sky_depth_state);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("basic_3d_atlas"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let white = BasicRenderer3d::make_atlas_bind(device, queue, &atlas_layout, &sampler, 1, 1, &[255, 255, 255, 255]);
        BasicRenderer3d {
            knobs: Basic3dOptions::default(),
            terrain_pipeline,
            sprite_pipeline,
            uniform_buffer,
            uniform_bind,
            atlas_layout,
            sampler,
            atlas_bind: white,
            billboard_atlas: None,
            terrain_vertices: GrowingBuffer::new("basic_3d_terrain_vertices", wgpu::BufferUsages::VERTEX),
            terrain_indices: GrowingBuffer::new("basic_3d_terrain_indices", wgpu::BufferUsages::INDEX),
            index_count: 0,
            height_scale: ViewOptions::default().height_scale,
            sprite_vertices: GrowingBuffer::new("basic_3d_sprite_vertices", wgpu::BufferUsages::VERTEX),
            depth: None,
            sky_pipeline,
            sky_vertices: GrowingBuffer::new("basic_3d_sky_vertices", wgpu::BufferUsages::VERTEX),
            sky: None,
        }
    }

    /// Put a sky image on the GPU, reusing the texture while the size holds.
    fn upload_sky(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, image: &SkyImage) {
        let stale = !matches!(&self.sky, Some(sky) if sky.width == image.width && sky.height == image.height);
        if stale {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("basic_3d_sky"),
                size: wgpu::Extent3d { width: image.width, height: image.height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: TARGET_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("basic_3d_sky"),
                layout: &self.atlas_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                ],
            });
            self.sky = Some(SkyTexture { texture, bind, width: image.width, height: image.height });
        }
        if let Some(sky) = &self.sky {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &sky.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                &image.rgba,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(image.width * 4), rows_per_image: Some(image.height) },
                wgpu::Extent3d { width: image.width, height: image.height, depth_or_array_layers: 1 },
            );
        }
    }

    /// Whether the last atlas handed over was real art rather than the white stand-in.
    pub fn has_atlas(&self) -> bool {
        self.billboard_atlas.is_some()
    }

    fn make_atlas_bind(device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler, width: u32, height: u32, rgba: &[u8]) -> wgpu::BindGroup {
        let texture = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("basic_3d_atlas"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: TARGET_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            rgba,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("basic_3d_atlas"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
            ],
        })
    }

    /// The depth buffer, remade whenever the target changes size.
    fn depth_view(&mut self, device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
        let stale = !matches!(&self.depth, Some((_, w, h)) if *w == width && *h == height);
        if stale {
            self.depth = None;
        }
        let (view, _, _) = self.depth.get_or_insert_with(|| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("basic_3d_depth"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            (texture.create_view(&wgpu::TextureViewDescriptor::default()), width, height)
        });
        view.clone()
    }
}

impl ViewportRenderer for BasicRenderer3d {
    fn name(&self) -> &'static str {
        "BasicRenderer3d"
    }

    fn world_changed(&mut self, ctx: &RenderContext, world: &World, options: &ViewOptions) -> Result<()> {
        self.height_scale = options.height_scale;
        let mesh = match self.knobs.mesh {
            MeshSource::Dem => build_dem_mesh(world, self.height_scale),
            MeshSource::Cells => build_cell_mesh(world, self.height_scale),
        };
        if mesh.indices.len() > u32::MAX as usize {
            return Err(anyhow!("the terrain mesh has too many indices"));
        }
        self.terrain_vertices.upload(ctx.device, ctx.queue, bytemuck::cast_slice(&mesh.vertices));
        self.terrain_indices.upload(ctx.device, ctx.queue, bytemuck::cast_slice(&mesh.indices));
        self.index_count = mesh.indices.len() as u32;
        log::info!("BasicRenderer3d: {} vertices, {} triangles from {:?}", mesh.vertices.len(), mesh.indices.len() / 3, self.knobs.mesh);
        Ok(())
    }

    fn atlas_changed(&mut self, ctx: &RenderContext, atlas: Option<Arc<SpriteAtlas>>) -> Result<()> {
        self.atlas_bind = match &atlas {
            Some(atlas) => BasicRenderer3d::make_atlas_bind(ctx.device, ctx.queue, &self.atlas_layout, &self.sampler, atlas.width, atlas.height, &atlas.rgba),
            None => BasicRenderer3d::make_atlas_bind(ctx.device, ctx.queue, &self.atlas_layout, &self.sampler, 1, 1, &[255, 255, 255, 255]),
        };
        self.billboard_atlas = atlas;
        Ok(())
    }

    fn options_ui(&mut self, ui: &mut egui::Ui) -> RendererUiOutcome {
        let mut rebuild = false;
        let mut redraw = false;
        ui.label("Terrain mesh");
        rebuild |= ui.radio_value(&mut self.knobs.mesh, MeshSource::Dem, "High resolution DEM").changed();
        rebuild |= ui.radio_value(&mut self.knobs.mesh, MeshSource::Cells, "Game map cells").changed();
        ui.add_space(4.0);
        redraw |= ui.add(egui::Slider::new(&mut self.knobs.sprite_scale, 0.005..=0.1).logarithmic(true).text("cells per sprite pixel")).changed();
        redraw |= ui.checkbox(&mut self.knobs.fog, "Distance fog").changed();
        redraw |= ui.checkbox(&mut self.knobs.show_trees, "Trees").changed();
        redraw |= ui.checkbox(&mut self.knobs.show_buildings, "Buildings").changed();
        redraw |= ui.checkbox(&mut self.knobs.show_units, "Units").changed();
        if self.billboard_atlas.is_none() {
            ui.colored_label(egui::Color32::from_rgb(220, 160, 60), "No sprite atlas: billboards are flat colour stand-ins.");
        }
        if rebuild {
            RendererUiOutcome::RebuildWorld
        } else if redraw {
            RendererUiOutcome::Redraw
        } else {
            RendererUiOutcome::Nothing
        }
    }

    fn render(
        &mut self,
        ctx: &RenderContext,
        encoder: &mut wgpu::CommandEncoder,
        target: &Target,
        camera: &Camera,
        world: &World,
        _options: &ViewOptions,
        sky: &mut dyn Skybox,
    ) -> Result<()> {
        // The sky first: whatever the skybox gives goes on the GPU and behind everything, and
        // the clear is black so the part it does not reach reads as uncovered, not as sky.
        let mut sky_count = 0u32;
        if let Some(image) = sky.image(camera) {
            let quad = sky_quad(image.width, image.height, target.width, target.height);
            self.sky_vertices.upload(ctx.device, ctx.queue, bytemuck::cast_slice(&quad));
            sky_count = quad.len() as u32;
            self.upload_sky(ctx.device, ctx.queue, image);
        }
        let clear = if sky_count > 0 { [0.0, 0.0, 0.0, 1.0] } else { SKY };

        let eye = camera.eye(world.map(), self.height_scale);
        let aspect = target.width as f32 / target.height.max(1) as f32;
        let view_proj = camera.view_projection(eye, aspect, 0.1, FAR);
        let (fog_start, fog_end) = if self.knobs.fog { (FOG_START, FOG_END) } else { (FAR * 10.0, FAR * 20.0) };
        let uniforms = Uniforms {
            view_proj: view_proj.to_columns(),
            fog_colour: SKY,
            fog: [fog_start, fog_end, 0.0, 0.0],
            eye: [eye[0], eye[1], eye[2], 1.0],
        };
        ctx.queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let boards = build_billboards(world, camera, self.height_scale, &self.knobs, self.billboard_atlas.as_deref());
        self.sprite_vertices.upload(ctx.device, ctx.queue, bytemuck::cast_slice(&boards));
        let sprite_count = boards.len() as u32;

        let depth = self.depth_view(ctx.device, target.width, target.height);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("basic_3d"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: clear[0] as f64, g: clear[1] as f64, b: clear[2] as f64, a: 1.0 }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_bind_group(0, &self.uniform_bind, &[]);
        if let (Some(vertices), Some(sky), true) = (&self.sky_vertices.buffer, &self.sky, sky_count > 0) {
            pass.set_pipeline(&self.sky_pipeline);
            pass.set_bind_group(1, &sky.bind, &[]);
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.draw(0..sky_count, 0..1);
        }
        pass.set_bind_group(1, &self.atlas_bind, &[]);
        if let (Some(vertices), Some(indices), true) = (&self.terrain_vertices.buffer, &self.terrain_indices.buffer, self.index_count > 0) {
            pass.set_pipeline(&self.terrain_pipeline);
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.index_count, 0, 0..1);
        }
        if let (Some(vertices), true) = (&self.sprite_vertices.buffer, sprite_count > 0) {
            pass.set_pipeline(&self.sprite_pipeline);
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.draw(0..sprite_count, 0..1);
        }
        Ok(())
    }
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::custom_gen::CustomParams;
    use crate::dem::DemSettings;

    fn small_world() -> World {
        let params = CustomParams { cols: 16, rows: 12, sprite_count: 4, building_count: 2, ..CustomParams::default() };
        World::custom(&params, DemSettings { factor: 2, ..DemSettings::default() }).expect("world")
    }

    #[test]
    fn the_dem_mesh_covers_every_sample() {
        let world = small_world();
        let mesh = build_dem_mesh(&world, 0.05);
        assert_eq!(mesh.vertices.len(), 32 * 24);
        assert_eq!(mesh.indices.len(), 31 * 23 * 6);
        assert!(mesh.indices.iter().all(|i| (*i as usize) < mesh.vertices.len()));
        assert!(mesh.vertices.iter().all(|v| v.pos[2] >= 0.0 && v.colour[0] <= 1.0));
    }

    #[test]
    fn the_cell_mesh_has_two_triangles_a_cell() {
        let world = small_world();
        let mesh = build_cell_mesh(&world, 0.05);
        assert_eq!(mesh.vertices.len(), 16 * 12 * 6);
        assert_eq!(mesh.indices.len(), 16 * 12 * 6);
    }

    #[test]
    fn billboards_face_the_camera_and_stand_on_the_ground() {
        let world = small_world();
        let camera = Camera { yaw: 0.0, ..Camera::default() };
        let options = ViewOptions::default();
        let knobs = Basic3dOptions::default();
        let boards = build_billboards(&world, &camera, options.height_scale, &knobs, None);
        let counts = world.map().counts();
        assert_eq!(boards.len(), (counts.trees + counts.buildings + world.sprites().len()) * 6);
        // With the camera looking along +x its right is -y, so a quad spans in y and not in x.
        let quad = &boards[..6];
        assert!(quad.iter().all(|v| (v.pos[0] - quad[0].pos[0]).abs() < 1e-5));
        assert!(quad.iter().any(|v| v.pos[1] != quad[0].pos[1]));
        assert!(quad.iter().any(|v| v.pos[2] > quad[1].pos[2]));
        let none = build_billboards(&world, &camera, options.height_scale, &Basic3dOptions { show_trees: false, show_buildings: false, show_units: false, ..knobs }, None);
        assert!(none.is_empty());
    }

    #[test]
    fn lighting_is_brightest_facing_the_sun() {
        assert!(lit(SUN) > lit([0.0, 0.0, 1.0]));
        assert!(lit([0.45, 0.35, -0.82]) >= 0.4);
        assert!((lit(SUN) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn the_sky_quad_sits_on_the_horizon_and_keeps_its_aspect() {
        // A 320x60 image across a 1280x800 target is 240 pixels tall: 0.6 of the top half.
        let quad = sky_quad(320, 60, 1280, 800);
        assert!(quad.iter().all(|v| v.pos[1] >= 0.0), "nothing below the horizon");
        let top = quad.iter().map(|v| v.pos[1]).fold(0.0f32, f32::max);
        assert!((top - 0.6).abs() < 1e-5, "{top}");
        assert!(quad.iter().any(|v| v.pos[0] == -1.0 && v.uv[0] == 0.0) && quad.iter().any(|v| v.pos[0] == 1.0 && v.uv[0] == 1.0));
        assert!(quad.iter().filter(|v| v.pos[1] == 0.0).all(|v| v.uv[1] == 1.0), "the bottom row of the image is the horizon");
    }

    #[test]
    fn the_shader_parses() {
        let module = wgpu::naga::front::wgsl::parse_str(SHADER).expect("shader parses");
        assert_eq!(module.entry_points.len(), 6, "terrain, sprites and sky, a vertex and fragment stage each");
        assert_eq!(std::mem::size_of::<Uniforms>(), 64 + 48);
    }
}
