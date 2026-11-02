// --------------------------------------------------------------------------------------------------------------------
// Getting a picture or a sound out as a file another program opens.
//
// The trait and the one way to run it, and nothing else. Each destination format is written
// beside the type it exports, so a type and everything that can be done to it stay in one
// file: Png in image.rs, Wav in audio.rs, Midi in music.rs.

use crate::error::{FormatError, Result};
use std::io::Write;
use std::path::Path;

/// Writes a T as some file format.
pub trait Exporter<T> {
    /// Without the dot: png, wav.
    const EXTENSION: &'static str;

    fn write(&self, item: &T, out: &mut dyn Write) -> Result<()>;

    fn to_bytes(&self, item: &T) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.write(item, &mut bytes)?;
        Ok(bytes)
    }
}

/// Export to a path, creating the file.
pub fn export_to_file<T, E: Exporter<T>>(exporter: &E, item: &T, path: &Path) -> Result<()> {
    let io = |source| FormatError::Io { path: path.to_path_buf(), source };
    let mut file = std::fs::File::create(path).map_err(io)?;
    exporter.write(item, &mut file)?;
    file.flush().map_err(io)
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::image::{Image, Png};

    /// Per-run test artefacts go under .test_output, which is gitignored.
    fn test_output() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(".test_output")
    }

    #[test]
    fn a_file_on_disk_holds_what_to_bytes_gives() {
        let out = test_output();
        std::fs::create_dir_all(&out).expect("the output folder");
        let path = out.join("export_to_file.png");
        let image = Image::from_pixels(2, 1, vec![[1, 2, 3, 255], [0, 0, 0, 0]]).expect("fits");
        export_to_file(&Png, &image, &path).expect("writes");
        assert_eq!(std::fs::read(&path).expect("reads back"), Png.to_bytes(&image).expect("encodes"));
        assert!(export_to_file(&Png, &image, &out.join("no").join("such").join("dir.png")).is_err());
    }
}
