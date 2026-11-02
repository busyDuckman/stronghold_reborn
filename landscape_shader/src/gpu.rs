// --------------------------------------------------------------------------------------------------------------------
// The GPU side: the device and surface, offscreen targets egui can show, and the one pipeline
// the top-down view needs. The render viewport is an offscreen target that is only cleared;
// what will draw into it is not decided here.

use crate::block_on::block_on;
use crate::top_down::{Camera, Geometry, Vertex};
use anyhow::{anyhow, Context as _, Result};
use bytemuck::{Pod, Zeroable};
use std::sync::Arc;
use winit::window::Window;

/// Offscreen targets and the DEM image use this; it is what egui expects to sample.
pub const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

// --------------------------------------------------------------------------------------------------------------------
// Device and surface

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
}

impl Gpu {
    /// Open the GPU for a window: an adapter that can present to it, a device, a configured surface.
    pub fn new(window: Arc<Window>) -> Result<Gpu> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = instance.create_surface(window.clone()).context("creating the window surface")?;
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .ok_or_else(|| anyhow!("no GPU adapter can present to this window"))?;
        let info = adapter.get_info();
        log::info!("GPU: {} ({:?}, {:?})", info.name, info.backend, info.device_type);
        let (device, queue) = block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("landscape_shader"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
            },
            None,
        ))
        .context("requesting the GPU device")?;
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| anyhow!("the surface is not supported by the adapter"))?;
        let caps = surface.get_capabilities(&adapter);
        if let Some(srgb) = caps.formats.iter().find(|f| f.is_srgb()) {
            config.format = *srgb;
        }
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&device, &config);
        log::info!("surface {}x{} {:?}", config.width, config.height, config.format);
        Ok(Gpu { device, queue, surface, config })
    }

    /// Follow a window resize. Zero-sized (minimised) is ignored; the surface keeps its size.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// The next frame to draw into.
    pub fn acquire(&self) -> Result<wgpu::SurfaceTexture> {
        match self.surface.get_current_texture() {
            Ok(frame) => Ok(frame),
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                log::warn!("surface lost or outdated, reconfiguring");
                self.surface.configure(&self.device, &self.config);
                self.surface.get_current_texture().context("acquiring a frame after reconfiguring")
            }
            Err(error) => Err(anyhow!("acquiring a frame: {error}")),
        }
    }
}

// --------------------------------------------------------------------------------------------------------------------
// Offscreen targets

/// A texture that a pass draws into and egui then shows as an image.
pub struct Target {
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
    pub egui_id: egui::TextureId,
    label: &'static str,
}

impl Target {
    pub fn new(device: &wgpu::Device, renderer: &mut egui_wgpu::Renderer, width: u32, height: u32, label: &'static str) -> Target {
        let (width, height) = (width.max(1), height.max(1));
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TARGET_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let egui_id = renderer.register_native_texture(device, &view, wgpu::FilterMode::Linear);
        Target { view, width, height, egui_id, label }
    }

    /// Recreate at a new size if it changed. Returns whether it did.
    pub fn ensure_size(&mut self, device: &wgpu::Device, renderer: &mut egui_wgpu::Renderer, width: u32, height: u32) -> bool {
        let (width, height) = (width.max(1), height.max(1));
        if self.width == width && self.height == height {
            return false;
        }
        renderer.free_texture(&self.egui_id);
        *self = Target::new(device, renderer, width, height, self.label);
        true
    }

    /// Clear the target to one colour and draw nothing else.
    pub fn clear(&self, encoder: &mut wgpu::CommandEncoder, colour: wgpu::Color) {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(self.label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.view,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(colour), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }
}

// --------------------------------------------------------------------------------------------------------------------
// The top-down pipeline

/// What the shader needs to place cell-unit geometry in a viewport.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct CameraUniform {
    cell_size: [f32; 2],
    origin: [f32; 2],
    viewport: [f32; 2],
    _pad: [f32; 2],
}

const SHADER: &str = r#"
struct Camera {
    cell_size: vec2<f32>,
    origin: vec2<f32>,
    viewport: vec2<f32>,
    pad: vec2<f32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) colour: vec4<f32>,
};
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) colour: vec4<f32>,
};

fn to_clip(cell_pos: vec2<f32>) -> vec4<f32> {
    let px = camera.origin + cell_pos * camera.cell_size;
    let ndc = vec2<f32>(px.x / camera.viewport.x * 2.0 - 1.0, 1.0 - px.y / camera.viewport.y * 2.0);
    return vec4<f32>(ndc, 0.0, 1.0);
}

@vertex
fn vs_flat(in: VsIn) -> VsOut {
    var out: VsOut;
    out.clip = to_clip(in.pos);
    out.colour = in.colour;
    return out;
}

@fragment
fn fs_flat(in: VsOut) -> @location(0) vec4<f32> {
    return in.colour;
}

// The DEM quad carries its texture coordinates in the colour's first two channels.
@group(1) @binding(0) var dem_texture: texture_2d<f32>;
@group(1) @binding(1) var dem_sampler: sampler;

@fragment
fn fs_textured(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(dem_texture, dem_sampler, in.colour.xy);
}
"#;

/// The DEM image on the GPU and the quad that shows it.
struct DemImage {
    bind: wgpu::BindGroup,
    quad: wgpu::Buffer,
}

pub struct TopDownPipeline {
    flat: wgpu::RenderPipeline,
    textured: wgpu::RenderPipeline,
    camera_buffer: wgpu::Buffer,
    camera_bind: wgpu::BindGroup,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    vertices: Option<wgpu::Buffer>,
    vertex_capacity: usize,
    vertex_count: u32,
    dem: Option<DemImage>,
}

impl TopDownPipeline {
    pub fn new(device: &wgpu::Device) -> TopDownPipeline {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("top_down"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("dem"),
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
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: camera_buffer.as_entire_binding() }],
        });
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
        };
        let make_pipeline = |label: &str, fragment: &str, layouts: &[&wgpu::BindGroupLayout]| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: layouts,
                push_constant_ranges: &[],
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_flat"),
                    buffers: &[vertex_layout.clone()],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: TARGET_FORMAT,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            })
        };
        let flat = make_pipeline("top_down_flat", "fs_flat", &[&camera_layout]);
        let textured = make_pipeline("top_down_dem", "fs_textured", &[&camera_layout, &texture_layout]);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("dem"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        TopDownPipeline {
            flat,
            textured,
            camera_buffer,
            camera_bind,
            texture_layout,
            sampler,
            vertices: None,
            vertex_capacity: 0,
            vertex_count: 0,
            dem: None,
        }
    }

    /// Upload new geometry, growing the buffer when it must.
    pub fn set_geometry(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, geometry: &Geometry) {
        let needed = geometry.vertices.len();
        if needed > self.vertex_capacity || self.vertices.is_none() {
            let capacity = needed.max(1024).next_power_of_two();
            self.vertices = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("top_down_vertices"),
                size: (capacity * std::mem::size_of::<Vertex>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
            self.vertex_capacity = capacity;
        }
        if let Some(buffer) = &self.vertices {
            if needed > 0 {
                queue.write_buffer(buffer, 0, bytemuck::cast_slice(&geometry.vertices));
            }
        }
        self.vertex_count = needed as u32;
    }

    /// Upload the DEM image (RGBA8, `cols` by `rows`) and the quad it is drawn on, `cells`
    /// wide and tall in cell units.
    pub fn set_dem(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, rgba: &[u8], cols: u32, rows: u32, cells: (f32, f32)) -> Result<()> {
        if rgba.len() != (cols as usize) * (rows as usize) * 4 {
            return Err(anyhow!("DEM image is {} bytes, expected {}x{}x4", rgba.len(), cols, rows));
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("dem"),
            size: wgpu::Extent3d { width: cols, height: rows, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TARGET_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(cols * 4), rows_per_image: Some(rows) },
            wgpu::Extent3d { width: cols, height: rows, depth_or_array_layers: 1 },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("dem"),
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        let (w, h) = cells;
        let corner = |x: f32, y: f32, u: f32, v: f32| Vertex { pos: [x, y], colour: [u, v, 0.0, 1.0] };
        let quad = [
            corner(0.0, 0.0, 0.0, 0.0),
            corner(w, 0.0, 1.0, 0.0),
            corner(w, h, 1.0, 1.0),
            corner(0.0, 0.0, 0.0, 0.0),
            corner(w, h, 1.0, 1.0),
            corner(0.0, h, 0.0, 1.0),
        ];
        let quad_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dem_quad"),
            size: std::mem::size_of_val(&quad) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&quad_buffer, 0, bytemuck::cast_slice(&quad));
        self.dem = Some(DemImage { bind, quad: quad_buffer });
        Ok(())
    }

    /// Draw into a target: clear, the DEM quad if asked and present, then the geometry.
    pub fn draw(&self, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, target: &Target, camera: &Camera, draw_dem: bool) {
        let uniform = CameraUniform {
            cell_size: [camera.cell_w, camera.cell_h],
            origin: [camera.origin_x, camera.origin_y],
            viewport: [target.width as f32, target.height as f32],
            _pad: [0.0; 2],
        };
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniform));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("top_down"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.02, g: 0.02, b: 0.025, a: 1.0 }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_bind_group(0, &self.camera_bind, &[]);
        if draw_dem {
            if let Some(dem) = &self.dem {
                pass.set_pipeline(&self.textured);
                pass.set_bind_group(1, &dem.bind, &[]);
                pass.set_vertex_buffer(0, dem.quad.slice(..));
                pass.draw(0..6, 0..1);
            }
        }
        if let (Some(vertices), true) = (&self.vertices, self.vertex_count > 0) {
            pass.set_pipeline(&self.flat);
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.draw(0..self.vertex_count, 0..1);
        }
    }
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests. The device and pipelines need a GPU and are exercised by running the app.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_uniform_is_the_size_the_shader_declares() {
        assert_eq!(std::mem::size_of::<CameraUniform>(), 32);
        assert_eq!(std::mem::size_of::<Vertex>(), 24);
    }

    #[test]
    fn the_shader_parses() {
        let module = wgpu::naga::front::wgsl::parse_str(SHADER).expect("shader parses");
        let names: Vec<&str> = module.entry_points.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"vs_flat") && names.contains(&"fs_flat") && names.contains(&"fs_textured"));
    }
}
