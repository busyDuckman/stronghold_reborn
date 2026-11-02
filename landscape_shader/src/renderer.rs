// --------------------------------------------------------------------------------------------------------------------
// The viewport renderer: what any rendering experiment implements to draw into the viewport.
//
// A renderer is handed the GPU, the offscreen target, the camera and the world every frame
// and is told when the world or the sprite atlas changes so it can rebuild what it keeps on
// the GPU. It also owns a strip of the sidebar for its own knobs. The app keeps a list of
// them and a combo box picks which one draws.

use crate::camera::Camera;
use crate::gpu::Target;
use crate::skybox::Skybox;
use crate::sprite_atlas::SpriteAtlas;
use crate::view_options::ViewOptions;
use crate::world::World;
use anyhow::Result;
use std::sync::Arc;

/// The GPU handles a renderer draws with.
pub struct RenderContext<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
}

/// What a renderer's sidebar strip asks for after the user touched it.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum RendererUiOutcome {
    Nothing,
    /// A knob changed that only affects drawing; the next frame shows it.
    Redraw,
    /// A knob changed that changes the GPU-side world; rebuild it.
    RebuildWorld,
}

pub trait ViewportRenderer {
    /// The name in the combo box.
    fn name(&self) -> &'static str;

    /// The world (or the height scale) changed: rebuild whatever depends on it.
    fn world_changed(&mut self, ctx: &RenderContext, world: &World, options: &ViewOptions) -> Result<()>;

    /// The sprite atlas changed, or went away: rebuild whatever depends on it.
    fn atlas_changed(&mut self, ctx: &RenderContext, atlas: Option<Arc<SpriteAtlas>>) -> Result<()>;

    /// This renderer's own controls, drawn in the sidebar's Renderer tab.
    fn options_ui(&mut self, ui: &mut egui::Ui) -> RendererUiOutcome;

    /// Draw one frame into the target. The skybox supplies what lies behind the terrain; a
    /// renderer asks it for the image and blits it as it pleases, or ignores it.
    fn render(
        &mut self,
        ctx: &RenderContext,
        encoder: &mut wgpu::CommandEncoder,
        target: &Target,
        camera: &Camera,
        world: &World,
        options: &ViewOptions,
        sky: &mut dyn Skybox,
    ) -> Result<()>;
}

// --------------------------------------------------------------------------------------------------------------------
// The empty renderer

/// Clears the viewport and draws nothing. The starting point for a new experiment.
pub struct EmptyRenderer;

impl ViewportRenderer for EmptyRenderer {
    fn name(&self) -> &'static str {
        "Empty"
    }

    fn world_changed(&mut self, _ctx: &RenderContext, _world: &World, _options: &ViewOptions) -> Result<()> {
        Ok(())
    }

    fn atlas_changed(&mut self, _ctx: &RenderContext, _atlas: Option<Arc<SpriteAtlas>>) -> Result<()> {
        Ok(())
    }

    fn options_ui(&mut self, ui: &mut egui::Ui) -> RendererUiOutcome {
        ui.label("Nothing to set. Clears the viewport and stops.");
        RendererUiOutcome::Nothing
    }

    fn render(
        &mut self,
        _ctx: &RenderContext,
        encoder: &mut wgpu::CommandEncoder,
        target: &Target,
        _camera: &Camera,
        _world: &World,
        _options: &ViewOptions,
        _sky: &mut dyn Skybox,
    ) -> Result<()> {
        target.clear(encoder, wgpu::Color { r: 0.05, g: 0.05, b: 0.07, a: 1.0 });
        Ok(())
    }
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests. Rendering needs a GPU; the trait's plumbing is exercised by the app.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_empty_renderer_has_a_name() {
        assert_eq!(EmptyRenderer.name(), "Empty");
    }
}
