// --------------------------------------------------------------------------------------------------------------------
// The application: a winit window, egui on wgpu, and the per-frame loop that ties the world,
// the options, the camera, the top-down pass and the chosen viewport renderer together.

use crate::backdrop::GameBackdrop;
use crate::basic_3d::BasicRenderer3d;
use crate::camera::{Camera, CameraInput};
use crate::dat::StrongDat;
use crate::gpu::{Gpu, Target, TopDownPipeline};
use crate::renderer::{EmptyRenderer, RenderContext, RendererUiOutcome, ViewportRenderer};
use crate::skybox::{NoSky, Skybox};
use crate::sprite_atlas::SpriteAtlas;
use crate::top_down::{build_geometry, dem_rgba, Camera as TopDownCamera};
use crate::ui::{self, RendererPanel, Textures, UiAction, UiState};
use crate::view_options::{TopDownBase, ViewOptions};
use crate::world::World;
use anyhow::{anyhow, Context as _, Result};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

/// The seed the app opens on: the game's own World 1.
const OPENING_SEED: u32 = 100;

/// Run the app until the window closes. Returns the error that stopped it early, if one did.
pub fn run() -> Result<()> {
    let event_loop = EventLoop::new().context("creating the event loop")?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App::default();
    event_loop.run_app(&mut app).context("running the event loop")?;
    match app.fatal.take() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// The winit handler. Everything that needs a window lives in `Running`, made on resume.
#[derive(Default)]
pub struct App {
    running: Option<Running>,
    fatal: Option<anyhow::Error>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_some() {
            return;
        }
        match Running::new(event_loop) {
            Ok(running) => self.running = Some(running),
            Err(error) => {
                log::error!("startup failed: {error:#}");
                self.fatal = Some(error);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(running) = &mut self.running else { return };
        let response = running.egui_state.on_window_event(&running.window, &event);
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => running.gpu.resize(size.width, size.height),
            WindowEvent::KeyboardInput { event, .. } => running.key_event(&event, response.consumed),
            WindowEvent::Focused(false) => running.keys = CameraInput::default(),
            WindowEvent::RedrawRequested => {
                if let Err(error) = running.frame() {
                    log::error!("frame failed: {error:#}");
                    self.fatal = Some(error);
                    event_loop.exit();
                    return;
                }
                if running.quit {
                    event_loop.exit();
                }
            }
            _ => {}
        }
        if response.repaint {
            running.window.request_redraw();
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }
}

// --------------------------------------------------------------------------------------------------------------------
// The running app

struct Running {
    window: Arc<Window>,
    gpu: Gpu,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    pipeline: TopDownPipeline,
    top_down_target: Target,
    render_target: Target,
    world: World,
    options: ViewOptions,
    ui: UiState,
    top_down_camera: TopDownCamera,
    camera: Camera,
    keys: CameraInput,
    last_frame: Instant,
    renderers: Vec<Box<dyn ViewportRenderer>>,
    /// Which renderers have not seen the current world or atlas.
    renderer_stale: Vec<bool>,
    selected_renderer: usize,
    /// What fills the viewport behind the terrain; the selected one is handed to the renderer.
    skyboxes: Vec<Box<dyn Skybox>>,
    selected_skybox: usize,
    dat: Option<StrongDat>,
    palettes: Vec<usize>,
    palette_entry: usize,
    atlas: Option<Arc<SpriteAtlas>>,
    dat_note: String,
    geometry_dirty: bool,
    dem_dirty: bool,
    quit: bool,
}

impl Running {
    fn new(event_loop: &ActiveEventLoop) -> Result<Running> {
        let attributes = Window::default_attributes().with_title("landscape_shader").with_inner_size(LogicalSize::new(1480.0, 880.0));
        let window = Arc::new(event_loop.create_window(attributes).context("creating the window")?);
        let gpu = Gpu::new(window.clone())?;
        let egui_ctx = egui::Context::default();
        let egui_state = egui_winit::State::new(egui_ctx.clone(), egui::ViewportId::ROOT, window.as_ref(), Some(window.scale_factor() as f32), None, None);
        let mut egui_renderer = egui_wgpu::Renderer::new(&gpu.device, gpu.config.format, None, 1, false);
        let pipeline = TopDownPipeline::new(&gpu.device);
        let top_down_target = Target::new(&gpu.device, &mut egui_renderer, 8, 8, "top_down_target");
        let render_target = Target::new(&gpu.device, &mut egui_renderer, 8, 8, "render_target");
        let options = ViewOptions::default();
        let world = World::stronghold(OPENING_SEED, options.dem)?;
        log::info!("opened on {}", world.title());
        let top_down_camera = TopDownCamera::fit(world.map().cols(), world.map().rows(), 8.0, 8.0, options.cell_aspect_2_3);
        let camera = Camera::at_map_centre(world.map());
        let renderers: Vec<Box<dyn ViewportRenderer>> = vec![Box::new(BasicRenderer3d::new(&gpu.device, &gpu.queue)), Box::new(EmptyRenderer)];
        let renderer_stale = vec![true; renderers.len()];

        // The game's art, if the container is on this machine; otherwise the billboards stand in.
        let (dat, palettes, palette_entry, dat_note) = match StrongDat::find() {
            Some(path) => match StrongDat::open(&path) {
                Ok(dat) => {
                    let palettes = dat.palette_indices();
                    let first = palettes.first().copied().unwrap_or(0);
                    let note = format!("{} entries from {}", dat.len(), path.display());
                    (Some(dat), palettes, first, note)
                }
                Err(error) => {
                    log::warn!("STRONG.DAT could not be read: {error:#}");
                    (None, Vec::new(), 0, format!("STRONG.DAT could not be read: {error:#}"))
                }
            },
            None => {
                log::warn!("STRONG.DAT not found under {}; billboards are flat colour stand-ins", crate::dat::STRONG_DAT_RELATIVE);
                (None, Vec::new(), 0, format!("STRONG.DAT not found under {}; billboards are stand-ins.", crate::dat::STRONG_DAT_RELATIVE))
            }
        };
        // The game's painted skies, when the container holds them; they are the opening choice.
        let mut skyboxes: Vec<Box<dyn Skybox>> = vec![Box::new(NoSky)];
        if let Some(dat) = &dat {
            match GameBackdrop::load(dat) {
                Ok(backdrop) => skyboxes.push(Box::new(backdrop)),
                Err(error) => log::warn!("the game backdrops could not be loaded: {error:#}"),
            }
        }
        let selected_skybox = skyboxes.len() - 1;
        let mut running = Running {
            window,
            gpu,
            egui_ctx,
            egui_state,
            egui_renderer,
            pipeline,
            top_down_target,
            render_target,
            world,
            options,
            ui: UiState::default(),
            top_down_camera,
            camera,
            keys: CameraInput::default(),
            last_frame: Instant::now(),
            renderers,
            renderer_stale,
            selected_renderer: 0,
            skyboxes,
            selected_skybox,
            dat,
            palettes,
            palette_entry,
            atlas: None,
            dat_note,
            geometry_dirty: true,
            dem_dirty: true,
            quit: false,
        };
        running.rebuild_atlas()?;
        Ok(running)
    }

    /// Decode the atlas under the chosen palette and hand it to every renderer next time it draws.
    fn rebuild_atlas(&mut self) -> Result<()> {
        self.atlas = match &self.dat {
            Some(dat) => {
                let atlas = SpriteAtlas::build(dat, self.palette_entry).context("building the sprite atlas")?;
                log::info!("{atlas:?}");
                Some(Arc::new(atlas))
            }
            None => None,
        };
        self.renderer_stale.iter_mut().for_each(|s| *s = true);
        Ok(())
    }

    /// Bring the selected renderer up to date with the world and the atlas if it is behind.
    fn refresh_selected_renderer(&mut self) -> Result<()> {
        let index = self.selected_renderer;
        if !self.renderer_stale.get(index).copied().unwrap_or(false) {
            return Ok(());
        }
        let ctx = RenderContext { device: &self.gpu.device, queue: &self.gpu.queue };
        let renderer = self.renderers.get_mut(index).ok_or_else(|| anyhow!("renderer {index} does not exist"))?;
        renderer.atlas_changed(&ctx, self.atlas.clone())?;
        renderer.world_changed(&ctx, &self.world, &self.options)?;
        self.renderer_stale[index] = false;
        Ok(())
    }

    /// Keep the held-key set current. Keys egui consumed (a text box has focus) are ignored.
    fn key_event(&mut self, event: &KeyEvent, consumed: bool) {
        let down = event.state == ElementState::Pressed;
        let PhysicalKey::Code(code) = event.physical_key else { return };
        if consumed && down {
            return;
        }
        match code {
            KeyCode::ArrowUp => self.keys.forward = down,
            KeyCode::ArrowDown => self.keys.back = down,
            KeyCode::ArrowLeft => self.keys.turn_left = down,
            KeyCode::ArrowRight => self.keys.turn_right = down,
            KeyCode::KeyQ => self.keys.strafe_left = down,
            KeyCode::KeyE => self.keys.strafe_right = down,
            KeyCode::PageUp => self.keys.rise = down,
            KeyCode::PageDown => self.keys.sink = down,
            KeyCode::Equal | KeyCode::NumpadAdd => self.keys.fov_wider = down,
            KeyCode::Minus | KeyCode::NumpadSubtract => self.keys.fov_narrower = down,
            KeyCode::ShiftLeft | KeyCode::ShiftRight => self.keys.fast = down,
            _ => {}
        }
    }

    /// A viewport rect in points to a target size in pixels; a fresh app has no rect yet.
    fn target_size(rect: Option<egui::Rect>, pixels_per_point: f32) -> (u32, u32) {
        match rect {
            Some(rect) => ((rect.width() * pixels_per_point).round().max(1.0) as u32, (rect.height() * pixels_per_point).round().max(1.0) as u32),
            None => (8, 8),
        }
    }

    /// One frame: move the camera, size the targets, refresh uploads, run the UI, act on it, draw.
    fn frame(&mut self) -> Result<()> {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        if self.keys.any() && !self.egui_ctx.wants_keyboard_input() {
            self.camera.apply(&self.keys, dt, self.world.map());
        }
        let pixels_per_point = self.egui_ctx.pixels_per_point();

        // Targets follow last frame's viewport rects, so egui never shows a texture this frame frees.
        let (tw, th) = Running::target_size(self.ui.top_down_rect, pixels_per_point);
        self.top_down_target.ensure_size(&self.gpu.device, &mut self.egui_renderer, tw, th);
        let (rw, rh) = Running::target_size(self.ui.render_rect, pixels_per_point);
        self.render_target.ensure_size(&self.gpu.device, &mut self.egui_renderer, rw, rh);
        let map = self.world.map();
        self.top_down_camera = TopDownCamera::fit(map.cols(), map.rows(), tw as f32, th as f32, self.options.cell_aspect_2_3);

        if self.dem_dirty {
            let dem = self.world.dem();
            let rgba = dem_rgba(dem);
            self.pipeline.set_dem(&self.gpu.device, &self.gpu.queue, &rgba, dem.cols() as u32, dem.rows() as u32, (map.cols() as f32, map.rows() as f32))?;
            self.dem_dirty = false;
        }
        if self.geometry_dirty {
            let geometry = build_geometry(&self.world, &self.options);
            self.pipeline.set_geometry(&self.gpu.device, &self.gpu.queue, &geometry);
            self.geometry_dirty = false;
        }
        self.refresh_selected_renderer()?;

        let raw_input = self.egui_state.take_egui_input(&self.window);
        let textures = Textures { top_down: self.top_down_target.egui_id, render: self.render_target.egui_id };
        let names: Vec<&'static str> = self.renderers.iter().map(|r| r.name()).collect();
        let selected = self.selected_renderer;
        let renderer = self.renderers.get_mut(selected).ok_or_else(|| anyhow!("renderer {selected} does not exist"))?;
        let sky_names: Vec<&'static str> = self.skyboxes.iter().map(|s| s.name()).collect();
        let sky_selected = self.selected_skybox;
        let skybox = self.skyboxes.get_mut(sky_selected).ok_or_else(|| anyhow!("skybox {sky_selected} does not exist"))?;
        let mut panel = RendererPanel {
            names,
            selected,
            palettes: &self.palettes,
            palette: self.palette_entry,
            dat_note: &self.dat_note,
            renderer: renderer.as_mut(),
            sky_names,
            sky_selected,
            skybox: skybox.as_mut(),
        };
        let (world, options, camera, ui_state, top_down_camera) = (&self.world, &mut self.options, &mut self.camera, &mut self.ui, self.top_down_camera);
        let mut ui_out = None;
        let mut full_output = self.egui_ctx.run(raw_input, |ctx| {
            ui_out = Some(ui::draw(ctx, world, options, camera, &mut panel, ui_state, textures, top_down_camera, pixels_per_point));
        });
        let platform_output = std::mem::take(&mut full_output.platform_output);
        self.egui_state.handle_platform_output(&self.window, platform_output);
        let ui_out = ui_out.ok_or_else(|| anyhow!("egui did not run the UI closure"))?;

        if ui_out.options_changed {
            self.geometry_dirty = true;
        }
        if ui_out.dem_changed {
            match self.world.rebuild_dem(self.options.dem) {
                Ok(()) => {
                    self.dem_dirty = true;
                    self.renderer_stale.iter_mut().for_each(|s| *s = true);
                }
                Err(error) => self.ui.error = Some(format!("Could not rebuild the DEM: {error:#}")),
            }
        }
        if ui_out.height_scale_changed || ui_out.renderer_outcome == Some(RendererUiOutcome::RebuildWorld) {
            self.renderer_stale.iter_mut().for_each(|s| *s = true);
        }
        if let Some(index) = ui_out.renderer_selected {
            if index < self.renderers.len() {
                self.selected_renderer = index;
            }
        }
        if let Some(index) = ui_out.sky_selected {
            if index < self.skyboxes.len() {
                self.selected_skybox = index;
            }
        }
        if let Some(entry) = ui_out.palette_selected {
            self.palette_entry = entry;
            if let Err(error) = self.rebuild_atlas() {
                self.ui.error = Some(format!("{error:#}"));
            }
        }
        if ui_out.camera_reset {
            self.camera = Camera { x: self.world.map().cols() as f32 * 0.5, y: self.world.map().rows() as f32 * 0.5, ..self.camera.clone() };
        }
        for action in ui_out.actions {
            self.apply(action);
        }
        self.refresh_selected_renderer()?;

        self.render(full_output)
    }

    /// Carry out one menu or dialog action. Failures go to the user, not the log alone.
    fn apply(&mut self, action: UiAction) {
        let result = match action {
            UiAction::Quit => {
                self.quit = true;
                Ok(())
            }
            UiAction::NewStronghold(seed) => self.replace_world(World::stronghold(seed, self.options.dem)),
            UiAction::NewStrongholdRandom => match clock_seed() {
                Ok(seed) => self.replace_world(World::stronghold(seed, self.options.dem)),
                Err(error) => Err(error),
            },
            UiAction::NewCustom(params) => self.replace_world(World::custom(&params, self.options.dem)),
        };
        if let Err(error) = result {
            log::warn!("{error:#}");
            self.ui.error = Some(format!("{error:#}"));
        }
    }

    fn replace_world(&mut self, world: Result<World>) -> Result<()> {
        let world = world?;
        log::info!("new world: {}", world.title());
        self.camera = Camera { x: world.map().cols() as f32 * 0.5, y: world.map().rows() as f32 * 0.5, ..self.camera.clone() };
        self.world = world;
        self.geometry_dirty = true;
        self.dem_dirty = true;
        self.renderer_stale.iter_mut().for_each(|s| *s = true);
        Ok(())
    }

    /// Draw the top-down pass, the chosen renderer into the render viewport, then egui over the window.
    fn render(&mut self, full_output: egui::FullOutput) -> Result<()> {
        let frame = match self.gpu.acquire() {
            Ok(frame) => frame,
            Err(error) => {
                // A missed frame is not fatal; the next redraw tries again.
                log::warn!("skipping a frame: {error:#}");
                return Ok(());
            }
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });

        self.pipeline.draw(&self.gpu.queue, &mut encoder, &self.top_down_target, &self.top_down_camera, self.options.base == TopDownBase::Dem);
        let ctx = RenderContext { device: &self.gpu.device, queue: &self.gpu.queue };
        let index = self.selected_renderer;
        let renderer = self.renderers.get_mut(index).ok_or_else(|| anyhow!("renderer {index} does not exist"))?;
        let sky_index = self.selected_skybox;
        let sky = self.skyboxes.get_mut(sky_index).ok_or_else(|| anyhow!("skybox {sky_index} does not exist"))?;
        renderer.render(&ctx, &mut encoder, &self.render_target, &self.camera, &self.world, &self.options, sky.as_mut())?;

        let paint_jobs = self.egui_ctx.tessellate(full_output.shapes, full_output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: [self.gpu.config.width, self.gpu.config.height], pixels_per_point: full_output.pixels_per_point };
        for (id, delta) in &full_output.textures_delta.set {
            self.egui_renderer.update_texture(&self.gpu.device, &self.gpu.queue, *id, delta);
        }
        let extra = self.egui_renderer.update_buffers(&self.gpu.device, &self.gpu.queue, &mut encoder, &paint_jobs, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.1, g: 0.1, b: 0.11, a: 1.0 }), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                })
                .forget_lifetime();
            self.egui_renderer.render(&mut pass, &paint_jobs, &screen);
        }
        for id in &full_output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
        self.gpu.queue.submit(extra.into_iter().chain(std::iter::once(encoder.finish())));
        frame.present();
        Ok(())
    }
}

/// A seed from the clock, the way the game's "Random" world takes one.
fn clock_seed() -> Result<u32> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).context("the clock is before 1970")?;
    Ok(now.as_millis() as u32)
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests. The window and GPU need a display; what can be checked headless is checked here.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_size_rounds_to_whole_pixels_and_never_zero() {
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::new(100.4, 0.2));
        assert_eq!(Running::target_size(Some(rect), 1.0), (100, 1));
        assert_eq!(Running::target_size(Some(rect), 2.0), (201, 1));
        assert_eq!(Running::target_size(None, 1.5), (8, 8));
    }

    #[test]
    fn clock_seed_is_available() {
        assert!(clock_seed().is_ok());
    }
}
