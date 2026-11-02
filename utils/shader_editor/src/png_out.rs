// --------------------------------------------------------------------------------------------------------------------
// Writing a rendered frame out as a PNG, first row first.
//
// Not macroquad's Image::export_png, which flips the rows vertically on the way out. That is
// right for the paired path, whose readback flips as well so the two cancel, and wrong for the
// unified one, which reads its targets back in memory order and has to keep them there: a
// baked texture that is upside down relative to the buffer it came from is a texture that
// samples upside down in the game.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

/// Write RGBA8 rows, row 0 first, to `path`. Creates nothing but the file itself, so the
/// directory has to exist, which is what `.output/` is for.
pub fn write_rgba(path: &str, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let want = (width as usize) * (height as usize) * 4;
    if rgba.len() != want {
        return Err(format!("{}: {} bytes for a {}x{} image, wanted {}", path, rgba.len(), width, height, want));
    }
    if let Some(dir) = Path::new(path).parent() {
        if !dir.as_os_str().is_empty() && !dir.exists() {
            return Err(format!("{}: {} does not exist", path, dir.display()));
        }
    }
    let file = File::create(path).map_err(|e| format!("{}: {}", path, e))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| format!("{}: {}", path, e))?;
    writer.write_image_data(rgba).map_err(|e| format!("{}: {}", path, e))
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrong_sized_buffer_is_refused_rather_than_written_short() {
        let e = write_rgba("/dev/null", 2, 2, &[0u8; 8]).expect_err("too few bytes");
        assert!(e.contains("wanted 16"), "{e}");
    }

    #[test]
    fn a_missing_directory_is_named_rather_than_guessed_at() {
        let e = write_rgba("no/such/dir/x.png", 1, 1, &[0u8; 4]).expect_err("no dir");
        assert!(e.contains("does not exist"), "{e}");
    }
}
