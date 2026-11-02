// --------------------------------------------------------------------------------------------------------------------
// The unified path's backend: naga compiles the GLSL, wgpu draws it into several targets, and
// the pixels come back to the CPU.
//
// This never presents. macroquad owns the window and the GL context; wgpu here asks for a
// primary backend (Vulkan, Metal, DX12), renders offscreen, and hands back an RGBA8 buffer
// that macroquad blits or that the headless path writes as a PNG. That is what lets a second
// GPU API live inside a tool built on the first one without the two fighting over a context,
// and it costs a readback per render. Fine: a bake is not a frame budget, and the render only
// happens when something changes.
//
// Two passes per render:
//   1. the shader's own stages, into one target per declared output
//   2. the generated view pass, reading those targets through the declared PBR roles
// The G-buffer is Rgba8Unorm, because that is what a baked texture is. A shader that needs
// more range than 0..1 out of a channel should scale it and say so in the channel's notes.

use std::collections::HashMap;

use macroquad::prelude::Vec2;
use wgpu::naga;

use crate::block_on::block_on;
use crate::unified::composite::{generate_final, ViewParams};
use crate::unified::pack::pack_block;
use crate::unified::parse::Unified;
use crate::values::UniformValues;

const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The most colour attachments the editor will set up. wgpu guarantees eight; a shader wanting
/// more than eight outputs has a different problem.
pub const MAX_TARGETS: usize = 8;

/// One rendered frame, in image order: row 0 is the top of the picture.
pub struct Rendered {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pub adapter: String,
}

impl Gpu {
    /// Ask for a device. Primary backends only: the GL backend would want the context
    /// macroquad is already holding, and sharing that is not a thing to find out about later.
    pub fn new() -> Result<Gpu, String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        })).ok_or_else(|| {
            "no Vulkan, Metal or DX12 adapter: the unified path needs one (the paired path does not)".to_string()
        })?;
        let info = adapter.get_info();
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("shader_editor"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            memory_hints: wgpu::MemoryHints::default(),
        }, None)).map_err(|e| format!("wgpu device: {e}"))?;

        Ok(Gpu {
            device,
            queue,
            adapter: format!("{} ({:?}, {:?})", info.name, info.backend, info.device_type),
        })
    }
}

// --------------------------------------------------------------------------------------------------------------------
// Compiling.

fn defines(shader: &Unified, stage_define: Option<&str>) -> naga::FastHashMap<String, String> {
    let mut out: naga::FastHashMap<String, String> = naga::FastHashMap::default();
    if let Some(d) = stage_define { out.insert(d.to_string(), "1".to_string()); }
    for t in shader.toggles.iter().filter(|t| t.on) {
        out.insert(t.define.clone(), "1".to_string());
    }
    out
}

/// Compile one stage through naga first, so a failure comes back as text with line numbers
/// rather than as a wgpu validation dump. The module wgpu then builds from the same source.
fn compile_stage(gpu: &Gpu, src: &str, stage: naga::ShaderStage, defs: naga::FastHashMap<String, String>, label: &str)
    -> Result<wgpu::ShaderModule, String>
{
    let mut frontend = naga::front::glsl::Frontend::default();
    let options = naga::front::glsl::Options { stage, defines: defs.clone() };
    let module = frontend.parse(&options, src)
        .map_err(|errs| format!("{} stage:\n{}", label, errs.emit_to_string(src)))?;
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator.validate(&module)
        .map_err(|e| format!("{} stage does not validate:\n{}", label, e.emit_to_string(src)))?;

    Ok(gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Glsl { shader: src.into(), stage, defines: defs },
    }))
}

/// wgpu's Display for a validation error is one line; everything useful is in the chain of
/// sources behind it, so walk that. A validation error you cannot read is a wasted afternoon.
fn detail(e: &wgpu::Error) -> String {
    let mut out = e.to_string();
    let mut source: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(e);
    while let Some(s) = source {
        out.push_str(&format!("\n  caused by: {}", s));
        source = s.source();
    }
    out
}

/// The stand-in vertex stage, for a shader that writes only a fragment stage. A fullscreen
/// triangle whose v = 0 lands on the first row of the target, matching the view pass.
///
/// GLSL rather than WGSL on purpose: the two front ends disagree about the default
/// interpolation sampling of a varying, and a vertex stage that does not match its fragment
/// stage is rejected by the pipeline. Compiling both halves the same way removes the question.
const BUILTIN_VERTEX: &str = r#"#version 450
layout(location = 0) out vec2 v_uv;

void main() {
    vec2 corner = vec2(float((gl_VertexIndex << 1) & 2), float(gl_VertexIndex & 2));
    v_uv = corner;
    gl_Position = vec4(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
}
"#;

/// A compiled shader, its uniform buffers and its two pipelines. Rebuilt whenever the file
/// changes or a toggle flips, which is the same thing the host's build would do.
pub struct Scene {
    shader: Unified,
    geometry: wgpu::RenderPipeline,
    geometry_bind: Option<wgpu::BindGroup>,
    block_buffers: Vec<wgpu::Buffer>,
    view: wgpu::RenderPipeline,
    view_layout: wgpu::BindGroupLayout,
    view_buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
    targets: Option<Targets>,
    pub used_builtin_vertex: bool,
}

struct Targets {
    size: (u32, u32),
    // held so the targets outlive the views that attach them, for as long as the size stands
    _textures: Vec<wgpu::Texture>,
    views: Vec<wgpu::TextureView>,
    composite: wgpu::Texture,
    composite_view: wgpu::TextureView,
    readback: wgpu::Buffer,
    bind: wgpu::BindGroup,
    padded_row: u32,
}

impl Scene {
    pub fn new(gpu: &Gpu, shader: &Unified, src: &str) -> Result<Scene, String> {
        if shader.outputs.len() > MAX_TARGETS {
            return Err(format!("{} fragment outputs; the editor sets up at most {}",
                shader.outputs.len(), MAX_TARGETS));
        }
        if let Some(b) = shader.blocks.iter().find(|b| b.group != 0) {
            return Err(format!("uniform block {} asks for set {}; naga puts every GLSL block in group 0",
                b.name, b.group));
        }

        let used_builtin_vertex = shader.vertex_define.is_none();
        let vs = if used_builtin_vertex {
            compile_stage(gpu, BUILTIN_VERTEX, naga::ShaderStage::Vertex,
                naga::FastHashMap::default(), "built-in vertex")?
        } else {
            compile_stage(gpu, src, naga::ShaderStage::Vertex,
                defines(shader, shader.vertex_define.as_deref()), "vertex")?
        };
        let fs = compile_stage(gpu, src, naga::ShaderStage::Fragment,
            defines(shader, shader.fragment_define.as_deref()), "fragment")?;

        // one uniform buffer per declared block, at the binding the shader asked for
        let mut entries = Vec::new();
        let mut binds = Vec::new();
        let mut block_buffers = Vec::new();
        for b in &shader.blocks {
            let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&b.name),
                size: b.size.max(16) as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: b.binding,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            });
            block_buffers.push(buffer);
        }
        for (i, b) in shader.blocks.iter().enumerate() {
            binds.push(wgpu::BindGroupEntry { binding: b.binding, resource: block_buffers[i].as_entire_binding() });
        }

        let geometry_layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blocks"),
            entries: &entries,
        });
        let geometry_bind = if entries.is_empty() { None } else {
            Some(gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("blocks"),
                layout: &geometry_layout,
                entries: &binds,
            }))
        };
        let groups: Vec<&wgpu::BindGroupLayout> = if entries.is_empty() { vec![] } else { vec![&geometry_layout] };
        let geometry_pl = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("geometry"),
            bind_group_layouts: &groups,
            push_constant_ranges: &[],
        });

        let colour_targets: Vec<Option<wgpu::ColorTargetState>> = shader.outputs.iter()
            .map(|_| Some(wgpu::ColorTargetState {
                format: TARGET_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            }))
            .collect();

        gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let geometry = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("geometry"),
            layout: Some(&geometry_pl),
            vertex: wgpu::VertexState {
                module: &vs,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &fs,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                targets: &colour_targets,
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        // The view pass. Its bindings are sampler, the View block, then one texture per target.
        let view_src = generate_final(&shader.roles, shader.outputs.len().max(1));
        let view_module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("view pass"),
            source: wgpu::ShaderSource::Wgsl(view_src.into()),
        });
        let mut view_entries = vec![
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ];
        for i in 0..shader.outputs.len().max(1) {
            view_entries.push(wgpu::BindGroupLayoutEntry {
                binding: i as u32 + 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
        }
        let view_layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("view"),
            entries: &view_entries,
        });
        let view_pl = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("view"),
            bind_group_layouts: &[&view_layout],
            push_constant_ranges: &[],
        });
        let view = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("view"),
            layout: Some(&view_pl),
            vertex: wgpu::VertexState {
                module: &view_module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &view_module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: TARGET_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        if let Some(e) = block_on(gpu.device.pop_error_scope()) {
            return Err(format!("pipeline: {}", detail(&e)));
        }

        let view_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("view params"),
            size: 48,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // repeat, so the tiled view shows a bake joining itself rather than clamping at the edge
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("view"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Ok(Scene {
            shader: shader.clone(),
            geometry,
            geometry_bind,
            block_buffers,
            view,
            view_layout,
            view_buffer,
            sampler,
            targets: None,
            used_builtin_vertex,
        })
    }

    fn ensure_targets(&mut self, gpu: &Gpu, w: u32, h: u32) {
        if self.targets.as_ref().map(|t| t.size) == Some((w, h)) { return; }
        let n = self.shader.outputs.len().max(1);
        let mut textures = Vec::new();
        let mut views = Vec::new();
        for i in 0..n {
            let tex = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&format!("target{}", i)),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: TARGET_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            views.push(tex.create_view(&Default::default()));
            textures.push(tex);
        }
        let composite = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("composite"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TARGET_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let composite_view = composite.create_view(&Default::default());

        // a texture copy's rows are padded to 256 bytes, so the readback buffer is wider than
        // the image and the padding is dropped on the way out
        let padded_row = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (padded_row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut entries = vec![
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            wgpu::BindGroupEntry { binding: 1, resource: self.view_buffer.as_entire_binding() },
        ];
        for (i, v) in views.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry { binding: i as u32 + 2, resource: wgpu::BindingResource::TextureView(v) });
        }
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("view"),
            layout: &self.view_layout,
            entries: &entries,
        });
        self.targets = Some(Targets {
            size: (w, h), _textures: textures, views, composite, composite_view, readback, bind, padded_row,
        });
    }

    /// Render the shader and the view pass, and bring the picture back.
    pub fn render(&mut self, gpu: &Gpu, values: &UniformValues, params: &ViewParams, w: u32, h: u32)
        -> Result<Rendered, String>
    {
        let (w, h) = (w.max(1), h.max(1));
        self.ensure_targets(gpu, w, h);
        let Some(targets) = self.targets.as_ref() else { return Err("no render targets".to_string()) };

        for (i, block) in self.shader.blocks.iter().enumerate() {
            let bytes = pack_block(block, values, Vec2::new(w as f32, h as f32));
            gpu.queue.write_buffer(&self.block_buffers[i], 0, &bytes);
        }
        gpu.queue.write_buffer(&self.view_buffer, 0, &params.bytes());

        gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let attachments: Vec<Option<wgpu::RenderPassColorAttachment>> = targets.views.iter()
                .map(|v| Some(wgpu::RenderPassColorAttachment {
                    view: v,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                }))
                .collect();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("geometry"),
                color_attachments: &attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.geometry);
            if let Some(bind) = &self.geometry_bind { pass.set_bind_group(0, bind, &[]); }
            pass.draw(0..self.shader.vertex_count, 0..1);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("view"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.composite_view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.view);
            pass.set_bind_group(0, &targets.bind, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &targets.composite,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &targets.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(targets.padded_row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        gpu.queue.submit([encoder.finish()]);

        let slice = targets.readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = gpu.device.poll(wgpu::Maintain::Wait);
        if let Some(e) = block_on(gpu.device.pop_error_scope()) {
            targets.readback.unmap();
            return Err(format!("render: {}", detail(&e)));
        }

        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        {
            let data = slice.get_mapped_range();
            let row = (w * 4) as usize;
            for y in 0..h as usize {
                let at = y * targets.padded_row as usize;
                rgba.extend_from_slice(&data[at..at + row]);
            }
        }
        targets.readback.unmap();
        Ok(Rendered { width: w, height: h, rgba })
    }
}

// --------------------------------------------------------------------------------------------------------------------
// Compiling without a GPU, for --check.

/// Compile both stages through naga and report what it says. No device, no pipeline, so this
/// works on a machine with no usable adapter, which is the case --check most needs to cover.
pub fn check_compile(shader: &Unified, src: &str) -> Vec<String> {
    let mut errs = Vec::new();
    let mut stages: Vec<(naga::ShaderStage, Option<&str>, &str)> = Vec::new();
    if shader.vertex_define.is_some() {
        stages.push((naga::ShaderStage::Vertex, shader.vertex_define.as_deref(), "vertex"));
    }
    stages.push((naga::ShaderStage::Fragment, shader.fragment_define.as_deref(), "fragment"));

    for (stage, define, label) in stages {
        let defs = defines(shader, define);
        let mut frontend = naga::front::glsl::Frontend::default();
        let options = naga::front::glsl::Options { stage, defines: defs };
        match frontend.parse(&options, src) {
            Err(e) => errs.push(format!("{} stage:\n{}", label, e.emit_to_string(src))),
            Ok(module) => {
                let mut validator = naga::valid::Validator::new(
                    naga::valid::ValidationFlags::all(),
                    naga::valid::Capabilities::all(),
                );
                if let Err(e) = validator.validate(&module) {
                    errs.push(format!("{} stage does not validate:\n{}", label, e.emit_to_string(src)));
                }
            }
        }
    }
    errs
}

/// The defines a stage is compiled with, for --check to print. Sorted, so the output is stable.
pub fn defines_for(shader: &Unified, stage_define: Option<&str>) -> Vec<String> {
    let map: HashMap<String, String> = defines(shader, stage_define).into_iter().collect();
    let mut out: Vec<String> = map.into_keys().collect();
    out.sort();
    out
}
