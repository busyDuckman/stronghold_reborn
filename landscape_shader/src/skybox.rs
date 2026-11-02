// --------------------------------------------------------------------------------------------------------------------
// The skybox: what fills the viewport behind the terrain.
//
// A skybox is asked for an image for the camera each frame and a renderer blits that image
// however it pleases. The contract is deliberately small so a real skybox (a cube map, a
// procedural sky, a day-night cycle) can take the place of the game's painted backdrops:
//   - the image spans the camera's horizontal field of view from its left edge to its right
//   - its bottom row is the horizon, which sits at eye level since the camera has no pitch
//   - its pixels are square, so a renderer keeps the aspect and may leave what the image
//     does not reach uncovered
// The app keeps a list of skyboxes and a combo box picks which one the renderer is handed.

use crate::camera::Camera;

/// An RGBA image, row major, top row first.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SkyImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl SkyImage {
    pub fn new(width: u32, height: u32) -> SkyImage {
        SkyImage { width, height, rgba: vec![0; (width * height * 4) as usize] }
    }
}

pub trait Skybox {
    /// The name in the combo box.
    fn name(&self) -> &'static str;

    /// This skybox's own controls, drawn in the sidebar. True when the image changed.
    fn options_ui(&mut self, ui: &mut egui::Ui) -> bool;

    /// The sky seen by this camera, or None for no sky at all. The image is owned by the
    /// skybox and stays valid until the next call, so a per-frame call costs no allocation.
    fn image(&mut self, camera: &Camera) -> Option<&SkyImage>;
}

// --------------------------------------------------------------------------------------------------------------------
// No sky

/// Draws nothing; the renderer's own clear colour shows.
pub struct NoSky;

impl Skybox for NoSky {
    fn name(&self) -> &'static str {
        "None"
    }

    fn options_ui(&mut self, ui: &mut egui::Ui) -> bool {
        ui.label("The renderer's own sky colour.");
        false
    }

    fn image(&mut self, _camera: &Camera) -> Option<&SkyImage> {
        None
    }
}
