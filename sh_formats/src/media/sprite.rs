// --------------------------------------------------------------------------------------------------------------------
// Sprites: a picture with the point the game positions it by, the animations they make up,
// and a set of animations keyed by what the thing is doing.
//
// What the reversing pass has established, and what it has not:
//   - a Placed record is drawn at (x - anchor_x, y - anchor_y): the anchor is the point in the
//     picture that lands on the position the caller names. Units carry it at the feet,
//     buildings hang off their footprint tile (about_assets.md, "The anchor").
//   - a unit frame carries an advance: how far the unit moves on that frame, so a walk cycle
//     covers ground on the frames where a foot is down (about_assets.md, "The byte at +5").
//   - a building is 3 views by up to 3 upgrades, pristine and damaged (dat_sprites.yaml).
//   - a creature has a table of frame sequences, each a run of consecutive entries. What each
//     sequence is (walk, attack) is not recovered, and levels are separate source files
//     (about_sprite_names.md, "Frame sets").
//   - the original stores one facing and mirrors at draw time (about_assets.md, the blit that
//     runs the string ops backwards), so facing is an operation here, not a stored state.

use crate::media::image::Image;
use std::collections::HashMap;
use std::hash::Hash;

/// A picture and the point in it that lands on the position it is drawn at.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Sprite {
    pub image: Image,
    pub anchor_x: i32,
    pub anchor_y: i32,
}

impl Sprite {
    /// Anchor at the top left, so the draw position is the corner.
    pub fn cornered(image: Image) -> Sprite {
        Sprite { image, anchor_x: 0, anchor_y: 0 }
    }

    /// The same sprite facing the other way, its anchor mirrored with it.
    pub fn mirrored(&self) -> Sprite {
        let anchor_x = self.image.width() as i32 - 1 - self.anchor_x;
        Sprite { image: self.image.flipped_horizontal(), anchor_x, anchor_y: self.anchor_y }
    }

    /// Where the top left corner goes when the sprite is drawn at (x, y).
    pub fn corner_when_drawn_at(&self, x: i32, y: i32) -> (i32, i32) {
        (x - self.anchor_x, y - self.anchor_y)
    }
}

/// One frame of an animation: the sprite and how far the thing moves on this frame.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Frame {
    pub sprite: Sprite,
    pub advance: u32,
}

/// A run of frames played in order. The frames carry the movement, so playing it at a
/// different rate changes how fast the thing moves; the two are not separable.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Animation {
    pub frames: Vec<Frame>,
}

impl Animation {
    pub fn new(frames: Vec<Frame>) -> Animation {
        Animation { frames }
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Frame n of a loop: past the end wraps round.
    pub fn frame(&self, n: usize) -> Option<&Frame> {
        if self.frames.is_empty() {
            None
        } else {
            self.frames.get(n % self.frames.len())
        }
    }

    /// The ground covered by one whole loop.
    pub fn advance_per_loop(&self) -> u32 {
        self.frames.iter().map(|f| f.advance).sum()
    }

    /// The whole animation facing the other way.
    pub fn mirrored(&self) -> Animation {
        Animation { frames: self.frames.iter().map(|f| Frame { sprite: f.sprite.mirrored(), advance: f.advance }).collect() }
    }

    /// Every frame side by side on one image, anchors on a common line, for looking at.
    pub fn strip(&self) -> Image {
        let left = self.frames.iter().map(|f| f.sprite.anchor_x).max().unwrap_or(0).max(0);
        let top = self.frames.iter().map(|f| f.sprite.anchor_y).max().unwrap_or(0).max(0);
        let cell_w = self.frames.iter().map(|f| f.sprite.image.width() as i32 - f.sprite.anchor_x).max().unwrap_or(0).max(0) + left;
        let cell_h = self.frames.iter().map(|f| f.sprite.image.height() as i32 - f.sprite.anchor_y).max().unwrap_or(0).max(0) + top;
        let mut strip = Image::blank(cell_w as usize * self.frames.len(), cell_h as usize);
        for (n, frame) in self.frames.iter().enumerate() {
            let (x, y) = frame.sprite.corner_when_drawn_at(n as i32 * cell_w + left, top);
            strip.blit(&frame.sprite.image, x as i64, y as i64);
        }
        strip
    }
}

// --------------------------------------------------------------------------------------------------------------------
// Sets of animations, keyed by what the thing is doing

/// A key for one animation in a set. Any small copyable value will do; two are provided.
pub trait SpriteState: Copy + Eq + Hash + std::fmt::Debug {}

/// The animations of one thing (a unit, a building), one per state.
#[derive(Clone, Debug)]
pub struct SpriteSet<S: SpriteState> {
    animations: HashMap<S, Animation>,
}

impl<S: SpriteState> Default for SpriteSet<S> {
    fn default() -> Self {
        SpriteSet { animations: HashMap::new() }
    }
}

impl<S: SpriteState> SpriteSet<S> {
    pub fn new() -> SpriteSet<S> {
        SpriteSet::default()
    }

    /// Set the animation for a state, replacing any there.
    pub fn insert(&mut self, state: S, animation: Animation) -> Option<Animation> {
        self.animations.insert(state, animation)
    }

    pub fn get(&self, state: S) -> Option<&Animation> {
        self.animations.get(&state)
    }

    pub fn states(&self) -> impl Iterator<Item = S> + '_ {
        self.animations.keys().copied()
    }

    pub fn len(&self) -> usize {
        self.animations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.animations.is_empty()
    }
}

/// A building as drawn: which of its three views, which upgrade, and whether it has decayed.
/// Each state is a single picture, held as a one-frame animation.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct BuildingState {
    /// 0 to 2. The sheet is cut three views per row.
    pub view: u8,
    /// 0 up; the chain length is per building, 1 to 3 in the CD build.
    pub upgrade: u8,
    /// The second sheet of nine, where the building has one.
    pub damaged: bool,
}

impl SpriteState for BuildingState {}

/// A unit as drawn: its level, and which of its frame sequences is playing.
/// What each sequence shows (walk, attack) is not recovered, so it is a number here; naming
/// them is for when the decompile reaches the code that picks one.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct UnitState {
    /// 1 up: the digit after the unit code in the source filename, 1fi1m to 1fi3m.
    pub level: u8,
    /// Which entry of the creature's frame descriptor table, from 0.
    pub sequence: u8,
}

impl SpriteState for UnitState {}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::image::Rgba;

    fn dot(width: usize, height: usize, x: usize, y: usize, colour: Rgba) -> Image {
        let mut image = Image::blank(width, height);
        image.set_pixel(x, y, colour);
        image
    }

    #[test]
    fn a_mirrored_sprite_keeps_its_anchor_on_the_same_pixel() {
        let sprite = Sprite { image: dot(4, 1, 1, 0, [9, 9, 9, 255]), anchor_x: 1, anchor_y: 0 };
        let mirrored = sprite.mirrored();
        assert_eq!(mirrored.anchor_x, 2);
        assert_eq!(mirrored.image.pixel(2, 0), Some([9, 9, 9, 255]));
        assert_eq!(sprite.corner_when_drawn_at(10, 10), (9, 10));
    }

    #[test]
    fn an_animation_loops_and_sums_its_advance() {
        let frame = |advance| Frame { sprite: Sprite::cornered(Image::blank(1, 1)), advance };
        let anim = Animation::new(vec![frame(0), frame(1), frame(2)]);
        assert_eq!(anim.frame(4).map(|f| f.advance), Some(1));
        assert_eq!(anim.advance_per_loop(), 3);
        assert!(Animation::default().frame(0).is_none());
        assert_eq!(anim.mirrored().len(), 3);
    }

    #[test]
    fn a_strip_lines_the_anchors_up() {
        let a = Sprite { image: dot(2, 2, 0, 1, [1, 1, 1, 255]), anchor_x: 0, anchor_y: 1 };
        let b = Sprite { image: dot(3, 1, 2, 0, [2, 2, 2, 255]), anchor_x: 2, anchor_y: 0 };
        let strip = Animation::new(vec![Frame { sprite: a, advance: 0 }, Frame { sprite: b, advance: 0 }]).strip();
        // A cell is the widest reach left of an anchor (b's 2) plus the widest reach right (a's 2),
        // by the tallest reach above (a's 1) plus below (1), so 4 by 2.
        assert_eq!((strip.width(), strip.height()), (8, 2));
        assert_eq!(strip.pixel(2, 1), Some([1, 1, 1, 255]), "a's anchor pixel sits on the anchor column and line");
        assert_eq!(strip.pixel(6, 1), Some([2, 2, 2, 255]), "b's anchor pixel sits on the same line one cell along");
    }

    #[test]
    fn a_set_is_keyed_by_state() {
        let mut set: SpriteSet<BuildingState> = SpriteSet::new();
        let state = BuildingState { view: 1, upgrade: 0, damaged: false };
        assert!(set.insert(state, Animation::default()).is_none());
        assert!(set.get(state).is_some());
        assert!(set.get(BuildingState { damaged: true, ..state }).is_none());
        assert_eq!(set.states().count(), 1);
        let mut units: SpriteSet<UnitState> = SpriteSet::new();
        units.insert(UnitState { level: 1, sequence: 0 }, Animation::default());
        assert_eq!(units.len(), 1);
    }
}
