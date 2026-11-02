// --------------------------------------------------------------------------------------------------------------------
// The game's painted skies as a skybox.
//
// The container holds nine backdrop sets after the dialog scenes: source files l1.lbm to
// l9.lbm, each cut into a 320x60 image, a 144-byte palette piece and a second 320x60 image.
// The owner's reading (dat_notes on entry 944) is that the piece is stamped over part of the
// main palette before the image is drawn, and that the game picked a set by some rule not
// yet recovered (direction, landscape or season are the guesses). Two things were checked
// here before building on it:
//   - the images only ever use colours 0..=207, and under the piece stamped at slots
//     160..=207 they use nearly every one of its 48 colours; at 208..=255 they come out as
//     noise. So the piece lands at 160, not at the end of the palette.
//   - the right edge of each set's second image continues into the left edge of its first,
//     and the first's right edge into the second's left, well above chance. The two images
//     are the halves of one 640-pixel loop: a full turn of horizon.
// Fixed mode draws the first image as a still; panorama mode cuts the camera's field of
// view out of the loop and pans it with the heading. Whether the loop really is a full turn
// is not recovered, so how many degrees it spans is a knob.

use crate::camera::Camera;
use crate::dat::{Palette, SpriteImage, StrongDat};
use crate::skybox::{SkyImage, Skybox};
use anyhow::{bail, Context, Result};

/// The main game palette by the owner's inspection (906 is the other candidate).
pub const MAIN_PALETTE: usize = 741;
/// The first entry of the first set; sets are three entries apart.
pub const FIRST_SET_ENTRY: usize = 944;
pub const SET_COUNT: usize = 9;
/// Where a set's palette piece is stamped.
pub const PIECE_SLOT: usize = 160;
/// The source file of each set, in container order.
const SET_NAMES: [&str; SET_COUNT] = ["l1", "l4", "l7", "l2", "l5", "l8", "l3", "l6", "l9"];

/// One set decoded: the loop of both halves, RGBA, and the width of one half.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BackdropSet {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub half_width: u32,
    pub rgba: Vec<u8>,
}

impl BackdropSet {
    /// Decode set number `set` (0..SET_COUNT) under the main palette with its piece stamped in.
    pub fn load(dat: &StrongDat, base: &Palette, set: usize) -> Result<BackdropSet> {
        if set >= SET_COUNT {
            bail!("backdrop set {set} is past the {SET_COUNT} in the container");
        }
        let first = FIRST_SET_ENTRY + set * 3;
        let piece = dat.payload(first + 1).context("the palette piece")?;
        let palette = base.stamped(PIECE_SLOT, piece).with_context(|| format!("stamping entry {}", first + 1))?;
        let a = dat.image(first)?;
        let b = dat.image(first + 2)?;
        let name = format!("{} ({first})", SET_NAMES[set]);
        BackdropSet::join(name, &a, &b, &palette)
    }

    /// Lay two images side by side as one loop.
    fn join(name: String, a: &SpriteImage, b: &SpriteImage, palette: &Palette) -> Result<BackdropSet> {
        if a.width != b.width || a.height != b.height || a.width == 0 || a.height == 0 {
            bail!("the halves of {name} are {}x{} and {}x{}, not a matching pair", a.width, a.height, b.width, b.height);
        }
        let (half, height) = (a.width as u32, a.height as u32);
        let width = half * 2;
        let mut rgba = vec![0u8; (width * height * 4) as usize];
        for (offset, image) in [(0, a), (half, b)] {
            for row in 0..height {
                for col in 0..half {
                    let i = (row * half + col) as usize;
                    let [r, g, b] = palette.colours[image.pixels[i] as usize];
                    let at = ((row * width + offset + col) * 4) as usize;
                    rgba[at..at + 4].copy_from_slice(&[r, g, b, 255]);
                }
            }
        }
        Ok(BackdropSet { name, width, height, half_width: half, rgba })
    }

    /// Copy `width` columns starting at `start` into out, wrapping round the loop.
    pub fn cut(&self, start: u32, width: u32, out: &mut SkyImage) {
        let width = width.max(1);
        if out.width != width || out.height != self.height {
            *out = SkyImage::new(width, self.height);
        }
        for row in 0..self.height {
            for col in 0..width {
                let from = ((row * self.width + (start + col) % self.width) * 4) as usize;
                let to = ((row * width + col) * 4) as usize;
                out.rgba[to..to + 4].copy_from_slice(&self.rgba[from..from + 4]);
            }
        }
    }
}

/// Which columns of a loop a camera sees: the first column and how many. The loop covers
/// loop_degrees of heading (a full turn if the game meant it that way, less if it did not),
/// so the count is the field of view's share of it, capped at the whole loop. Heading 0
/// (east) looks at the middle of the first half; turning right pans the sky left, and past
/// the loop's span it wraps.
pub fn window(loop_width: u32, loop_degrees: f32, yaw: f32, fov_degrees: f32) -> (u32, u32) {
    let per_loop = loop_width as f32;
    let loop_degrees = loop_degrees.max(1.0);
    let width = (per_loop * fov_degrees / loop_degrees).round().clamp(1.0, per_loop) as u32;
    let centre = per_loop * 0.25 + yaw.to_degrees() / loop_degrees * per_loop;
    let start = (centre - width as f32 * 0.5).rem_euclid(per_loop);
    (start.round() as u32 % loop_width.max(1), width)
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum BackdropMode {
    /// The first half as a still, whatever the camera does.
    Fixed,
    /// The field of view cut from the loop, panning with the heading.
    Panorama,
}

pub struct GameBackdrop {
    sets: Vec<BackdropSet>,
    set: usize,
    mode: BackdropMode,
    /// How much heading one trip round the loop covers, in panorama mode.
    loop_degrees: f32,
    scratch: SkyImage,
}

impl GameBackdrop {
    /// Decode every set. Fails if the container does not hold them as expected.
    pub fn load(dat: &StrongDat) -> Result<GameBackdrop> {
        let base = dat.palette(MAIN_PALETTE).context("the main palette")?;
        let sets = (0..SET_COUNT).map(|set| BackdropSet::load(dat, &base, set)).collect::<Result<Vec<_>>>()?;
        Ok(GameBackdrop::from_sets(sets))
    }

    pub fn from_sets(sets: Vec<BackdropSet>) -> GameBackdrop {
        GameBackdrop { sets, set: 0, mode: BackdropMode::Fixed, loop_degrees: 360.0, scratch: SkyImage::default() }
    }

    pub fn set_count(&self) -> usize {
        self.sets.len()
    }
}

impl Skybox for GameBackdrop {
    fn name(&self) -> &'static str {
        "Game backdrop"
    }

    fn options_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        let mut set = self.set;
        egui::ComboBox::from_label("set").selected_text(self.sets.get(set).map(|s| s.name.as_str()).unwrap_or("?")).show_ui(ui, |ui| {
            for (i, s) in self.sets.iter().enumerate() {
                ui.selectable_value(&mut set, i, &s.name);
            }
        });
        if set != self.set {
            self.set = set;
            changed = true;
        }
        changed |= ui.radio_value(&mut self.mode, BackdropMode::Fixed, "Fixed: the first half, still").changed();
        changed |= ui.radio_value(&mut self.mode, BackdropMode::Panorama, "Panorama: both halves joined, pans with the heading").changed();
        if self.mode == BackdropMode::Panorama {
            changed |= ui.add(egui::Slider::new(&mut self.loop_degrees, 45.0..=360.0).text("degrees per loop")).changed();
        }
        ui.label(format!("Palette {MAIN_PALETTE} with the set's 48 colours stamped at slot {PIECE_SLOT}."));
        changed
    }

    fn image(&mut self, camera: &Camera) -> Option<&SkyImage> {
        let set = self.sets.get(self.set)?;
        let (start, width) = match self.mode {
            BackdropMode::Fixed => (0, set.half_width),
            BackdropMode::Panorama => window(set.width, self.loop_degrees, camera.yaw, camera.fov_degrees),
        };
        set.cut(start, width, &mut self.scratch);
        Some(&self.scratch)
    }
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4x1 half: colours 1,2,3,4 on the left, 5,6,7,8 on the right; the palette maps n to (n, 0, 0).
    fn tiny_set() -> BackdropSet {
        let mut palette = Palette { colours: [[0; 3]; 256] };
        for n in 0..9u8 {
            palette.colours[n as usize] = [n, 0, 0];
        }
        let half = |first: u8| SpriteImage { width: 4, height: 1, anchor_x: 0, anchor_y: 0, kind: 1, pixels: (first..first + 4).collect(), mask: vec![true; 4] };
        BackdropSet::join("tiny".into(), &half(1), &half(5), &palette).expect("joins")
    }

    fn reds(image: &SkyImage) -> Vec<u8> {
        image.rgba.chunks(4).map(|p| p[0]).collect()
    }

    #[test]
    fn the_halves_join_into_one_loop() {
        let set = tiny_set();
        assert_eq!((set.width, set.height, set.half_width), (8, 1, 4));
        let mut out = SkyImage::default();
        set.cut(0, 4, &mut out);
        assert_eq!(reds(&out), vec![1, 2, 3, 4]);
        set.cut(6, 4, &mut out);
        assert_eq!(reds(&out), vec![7, 8, 1, 2], "wraps round the loop");
        set.cut(3, 8, &mut out);
        assert_eq!(reds(&out), vec![4, 5, 6, 7, 8, 1, 2, 3]);
    }

    #[test]
    fn the_window_follows_the_heading_and_the_fov() {
        let (start, width) = window(640, 360.0, 0.0, 90.0);
        assert_eq!(width, 160, "a quarter turn is a quarter of the loop");
        assert_eq!(start, 80, "east looks at the middle of the first half");
        let (turned, _) = window(640, 360.0, std::f32::consts::FRAC_PI_2, 90.0);
        assert_eq!(turned, 240, "a right turn moves the window on");
        let (_, wide) = window(640, 360.0, 0.0, 360.0);
        assert_eq!(wide, 640);
        let (wrapped, _) = window(640, 360.0, -std::f32::consts::FRAC_PI_2, 90.0);
        assert_eq!(wrapped, 560, "a left turn wraps below zero");
        let (_, short) = window(640, 180.0, 0.0, 90.0);
        assert_eq!(short, 320, "a loop that spans half a turn shows twice as much per degree");
        let (_, capped) = window(640, 45.0, 0.0, 90.0);
        assert_eq!(capped, 640, "a field of view wider than the loop shows all of it");
    }

    #[test]
    fn a_mismatched_pair_is_refused() {
        let palette = Palette { colours: [[0; 3]; 256] };
        let a = SpriteImage { width: 4, height: 1, anchor_x: 0, anchor_y: 0, kind: 1, pixels: vec![0; 4], mask: vec![true; 4] };
        let b = SpriteImage { width: 3, height: 1, anchor_x: 0, anchor_y: 0, kind: 1, pixels: vec![0; 3], mask: vec![true; 3] };
        assert!(BackdropSet::join("odd".into(), &a, &b, &palette).is_err());
    }

    #[test]
    fn the_skybox_serves_a_still_or_the_view() {
        let mut sky = GameBackdrop::from_sets(vec![tiny_set()]);
        let camera = Camera { yaw: 0.0, fov_degrees: 180.0, ..Camera::default() };
        assert_eq!(reds(sky.image(&camera).expect("image")), vec![1, 2, 3, 4]);
        sky.mode = BackdropMode::Panorama;
        assert_eq!(reds(sky.image(&camera).expect("image")), vec![1, 2, 3, 4], "half a turn centred on the first half");
        let turned = Camera { yaw: std::f32::consts::PI, ..camera };
        assert_eq!(reds(sky.image(&turned).expect("image")), vec![5, 6, 7, 8]);
    }

    #[test]
    fn the_real_sets_load_when_the_container_is_here() {
        let Some(path) = StrongDat::find() else {
            println!("STRONG.DAT not on this machine; skipping");
            return;
        };
        let dat = StrongDat::open(&path).expect("opens");
        let sky = GameBackdrop::load(&dat).expect("loads");
        assert_eq!(sky.set_count(), SET_COUNT);
        assert_eq!((sky.sets[0].width, sky.sets[0].height), (640, 60));
        assert_eq!(sky.sets[0].name, "l1 (944)");
    }
}
