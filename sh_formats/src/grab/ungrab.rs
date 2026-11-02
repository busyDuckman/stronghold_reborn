// --------------------------------------------------------------------------------------------------------------------
// Ungrab: STRONG.DAT back into the source tree STRONG.EXE grab builds it from.
//
// The manifest (grab/manifest.rs) says which file and operation made every entry; this inverts each one: pixels back
// at their rectangles, the load's edits undone, markers and the font laid out where the packer's scans find them,
// palettes into the CMAP, audio back into VOC and XMI files, raw copies stripped of what grab appends. The tree is
// re-cut against the container before it is handed back, and reversing/scripts/ungrab_check.py runs the original
// packer over it to prove the full circle.

use crate::dat::{DatFile, NameTable, PaletteResolver, MAIN_PALETTE_ENTRY};
use crate::error::{FormatError, Result};
use crate::formats::image::IndexedImage;
use crate::file_format::FileFormat;
use crate::grab::lbm::{Lbm, CMAP_LEN};
use crate::grab::manifest::{Edit, Manifest, Op};
use crate::grab::md5::md5_hex;
use crate::grab::sheet::{edit_pixel, rotate_pixel, unedit_cmap, unedit_pixel, FrameCut, Sheet, Why, SCREEN};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// The German floppy build's STRONG.DAT: a different build whose packer was never traced.
pub const GERMAN_FLOPPY_MD5: &str = "f7a424df65f5e69b1a1172eb439fb2e3";
/// What 3000:cbd0 appends to a driver when its second argument is set.
pub const DRIVER_PAD: usize = 15;
/// A kind 3 record's header: kind, far pointer, size word, four bytes never written, rate word.
pub const AUDIO_HEADER: usize = 13;
pub const AUDIO_KIND: u8 = 3;
/// The bytes of a kind 3 record 3000:c79c never writes: heap, whatever it held.
pub const AUDIO_UNWRITTEN: std::ops::Range<usize> = 7..11;
/// What the packer reads of a VOC before the samples: 26 bytes of file header and block 1's six.
pub const VOC_HEADER: usize = 32;
/// The instructions and the entry map written beside the sources, both 8.3 names grab never opens.
pub const README_NAME: &str = "ungrab.txt";
pub const MAP_NAME: &str = "ungrab.tsv";

const README_TEMPLATE: &str = include_str!("ungrab_readme.txt");

/// How the archive was recognised.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Recognised {
    /// Its md5 is the English CD build's, the container the manifest was traced against.
    Shipped,
    /// An unknown md5 whose every entry still has the kind and shape the packer makes, allowed by the caller.
    ByLayout { md5: String },
}

/// The source tree: every file grab opens, by its DOS name, plus the instructions and the map.
pub struct SourceTree {
    /// DOS name, case folded, as grab opens it, to the file's bytes.
    pub files: BTreeMap<String, Vec<u8>>,
    pub recognised: Recognised,
    /// How many of the files are pictures.
    pub pictures: usize,
    pub readme: String,
    pub map: String,
}

impl std::fmt::Debug for SourceTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SourceTree {} files, {} pictures, {:?}", self.files.len(), self.pictures, self.recognised)
    }
}

// --------------------------------------------------------------------------------------------------------------------
// Recognising the archive

/// Refuse anything but the container the manifest describes, unless the caller accepts a matching layout.
fn recognise(dat: &[u8], manifest: &Manifest, accept_layout: bool) -> Result<Recognised> {
    let md5 = md5_hex(dat);
    if md5 == manifest.dat_md5 {
        if dat.len() != manifest.dat_size {
            return Err(FormatError::Ungrab(format!("md5 {md5} matches but the size {} is not {}", dat.len(), manifest.dat_size)));
        }
        return Ok(Recognised::Shipped);
    }
    if md5 == GERMAN_FLOPPY_MD5 {
        return Err(FormatError::Ungrab(format!("this is the German floppy STRONG.DAT (md5 {md5}), a different build; its STRONG.EXE's packer was never traced, so a tree made from the English CD manifest is not what that grab reads; refused")));
    }
    if !accept_layout {
        return Err(FormatError::Ungrab(format!("unknown STRONG.DAT (md5 {md5}); this ungrab knows the English CD build, md5 {}. A container rebuilt by STRONG.EXE grab from an ungrab tree can be ungrabbed with --accept-layout, which still refuses any entry the packer could not have made", manifest.dat_md5)));
    }
    log::warn!("STRONG.DAT md5 {md5} is not the shipped one; ungrabbing by layout, every entry checked against the manifest");
    Ok(Recognised::ByLayout { md5 })
}

// --------------------------------------------------------------------------------------------------------------------
// The rebuild

/// Turn a STRONG.DAT into the tree grab rebuilds it from, or say exactly why not.
pub fn ungrab(dat: &[u8], accept_layout: bool) -> Result<SourceTree> {
    let manifest = Manifest::embedded()?;
    let recognised = recognise(dat, &manifest, accept_layout)?;
    let archive = DatFile::from_bytes(dat.to_vec(), PathBuf::from("STRONG.DAT"), NameTable::empty())?;
    if archive.entry_count() != manifest.ops.len() {
        return Err(FormatError::Ungrab(format!("{} entries; the manifest describes {}", archive.entry_count(), manifest.ops.len())));
    }
    let mut sheets: BTreeMap<String, Sheet> = BTreeMap::new();
    let mut frames: HashMap<String, Vec<FrameCut>> = HashMap::new();
    let mut audio: BTreeMap<String, Vec<(usize, &[u8])>> = BTreeMap::new();
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut first_image: HashMap<String, usize> = HashMap::new();
    for (n, op) in manifest.ops.iter().enumerate() {
        let pay = archive.payload_bytes(n)?;
        let refuse = |why: String| FormatError::Ungrab(format!("entry {n} ({} from {}): {why}", op.how(), op.file()));
        match op {
            Op::Cut { file, kind, rect, load } => {
                if pay.first() != Some(kind) {
                    return Err(refuse(format!("kind {:?} in the archive, the packer makes kind {kind}", pay.first())));
                }
                let image = IndexedImage::read(pay).map_err(|e| refuse(e.to_string()))?;
                if (image.width(), image.height()) != (rect.width(), rect.height()) {
                    return Err(refuse(format!("{}x{}, cut from a {}x{} rectangle", image.width(), image.height(), rect.width(), rect.height())));
                }
                let sheet = sheets.entry(file.clone()).or_insert_with(|| Sheet::new(file));
                sheet.loads.insert(load.x);
                sheet.rotated |= load.edits.contains(&Edit::Rotate);
                first_image.entry(file.clone()).or_insert(n);
                for y in 0..image.height() {
                    for x in 0..image.width() {
                        let v = unedit_pixel(image.pixels[y * image.width() + x], &load.edits).ok_or_else(|| refuse(format!("pixel ({x},{y}) is a colour the load's edits never leave")))?;
                        sheet.put(rect.x0 + x + load.x, rect.y0 + y, v, Why::Entry(n))?;
                    }
                }
                if *kind == 4 {
                    frames.entry(file.clone()).or_default().push(FrameCut { rect: *rect, advance: pay[5], ordinal: n });
                }
            }
            Op::Palette { file, at, size, edits } => {
                if pay.len() != *size {
                    return Err(refuse(format!("{} bytes, the packer stashes {size}", pay.len())));
                }
                let sheet = sheets.entry(file.clone()).or_insert_with(|| Sheet::new(file));
                sheet.rotated |= edits.contains(&Edit::Rotate);
                for (i, b) in pay.iter().enumerate() {
                    if *b >= 64 {
                        return Err(refuse(format!("byte {i} is {b}, past 6 bits")));
                    }
                    sheet.put_cmap(unedit_cmap(at + i, edits), b << 2, n)?;
                }
            }
            Op::Font { file } => {
                let sheet = sheets.entry(file.clone()).or_insert_with(|| Sheet::new(file));
                sheet.font = Some((n, pay.to_vec()));
            }
            Op::Samples { file, load, offsets } => {
                if pay.len() != offsets.len() {
                    return Err(refuse(format!("{} bytes for {} sampled offsets", pay.len(), offsets.len())));
                }
                let sheet = sheets.entry(file.clone()).or_insert_with(|| Sheet::new(file));
                for (k, off) in offsets.iter().enumerate() {
                    let v = unedit_pixel(pay[k], &load.edits).ok_or_else(|| refuse(format!("byte {k} cannot come from a pixel")))?;
                    sheet.put(off % SCREEN + load.x, off / SCREEN, v, Why::Sample { entry: n, byte: k })?;
                }
            }
            Op::Sound { file } => audio.entry(file.clone()).or_default().push((n, pay)),
            Op::Copy { file, pad } => {
                let body = if *pad {
                    let cut = pay.len().checked_sub(DRIVER_PAD).ok_or_else(|| refuse("shorter than the pad".into()))?;
                    if pay[cut..].iter().any(|b| *b != 0) {
                        return Err(refuse(format!("does not end in {DRIVER_PAD} zero bytes")));
                    }
                    &pay[..cut]
                } else {
                    pay
                };
                if files.get(file).is_some_and(|old| old.as_slice() != body) {
                    return Err(refuse("stashed twice with different bytes".into()));
                }
                files.insert(file.clone(), body.to_vec());
            }
        }
    }
    for anim in &manifest.anims {
        let sheet = sheets.get_mut(&anim.file).ok_or_else(|| FormatError::Ungrab(format!("animation sheet {} has no cuts", anim.file)))?;
        sheet.place_markers(anim.load_x, &anim.descriptors, frames.get(&anim.file).map(|f| f.as_slice()).unwrap_or(&[]))?;
    }
    for (name, records) in &audio {
        files.insert(name.clone(), rebuild_audio(name, records)?);
    }
    let pictures = sheets.len();
    for (name, sheet) in sheets.iter_mut() {
        sheet.place_font()?;
        let display = display_cmap(&archive, first_image.get(name).copied(), sheet.rotated)?;
        let lbm = sheet.compile(&display)?;
        let compress = lbm.width > SCREEN;
        let bytes = lbm.to_bytes(compress).map_err(|e| FormatError::Ungrab(format!("{name}: {e}")))?;
        if files.insert(name.clone(), bytes).is_some() {
            return Err(FormatError::Ungrab(format!("{name} is both a picture and a copied file")));
        }
    }
    for name in [README_NAME, MAP_NAME] {
        if files.contains_key(name) {
            return Err(FormatError::Ungrab(format!("grab opens a file called {name}, which the ungrab writes its notes to")));
        }
    }
    let checked = recut(&archive, &manifest, &files)?;
    log::info!("ungrab: {} files, {pictures} pictures; {checked} of {} entries re-cut from the tree and identical", files.len(), manifest.ops.len());
    let readme = readme(&manifest, &recognised, files.len(), pictures, checked, dat);
    let map = entry_map(&manifest);
    Ok(SourceTree { files, recognised, pictures, readme, map })
}

/// The first VOC time constant 3000:c775 turns into this rate word: 1000000 / (256 - tc), 16 bits.
fn voc_time_constant(word: u16) -> Option<u8> {
    (0..=255u8).find(|tc| voc_rate_word(*tc) == word)
}

fn voc_rate_word(tc: u8) -> u16 {
    (1_000_000u32 / (256 - tc as u32)) as u16
}

/// One audio file from every record made of it; the records must all agree on what the file was.
fn rebuild_audio(name: &str, records: &[(usize, &[u8])]) -> Result<Vec<u8>> {
    let mut built: Option<Vec<u8>> = None;
    for (n, pay) in records {
        let refuse = |why: String| FormatError::Ungrab(format!("entry {n} (audio from {name}): {why}"));
        if pay.len() < AUDIO_HEADER || pay[0] != AUDIO_KIND || pay[1..5] != [0, 0, 0, 0] {
            return Err(refuse("not a kind 3 record with a null far pointer".into()));
        }
        let size = u16::from_le_bytes([pay[5], pay[6]]) as usize;
        if AUDIO_HEADER + size != pay.len() {
            return Err(refuse(format!("size word {size} against {} bytes", pay.len())));
        }
        let data = &pay[AUDIO_HEADER..];
        let word = u16::from_le_bytes([pay[11], pay[12]]);
        let file = if name.ends_with(".xmi") {
            if word != 0 {
                return Err(refuse(format!("an .xmi record with rate word {word}")));
            }
            data.to_vec()
        } else {
            let tc = voc_time_constant(word).ok_or_else(|| refuse(format!("no VOC time constant gives rate word {word}")))?;
            let length = (data.len() + 2) as u32;
            let mut voc = b"Creative Voice File\x1a".to_vec();
            for w in [26u16, 0x010a, 0x1129] {
                voc.extend_from_slice(&w.to_le_bytes());
            }
            voc.push(1);
            voc.extend_from_slice(&length.to_le_bytes()[..3]);
            voc.extend_from_slice(&[tc, 0]);
            voc.extend_from_slice(data);
            voc.push(0);
            voc
        };
        if built.as_ref().is_some_and(|b| *b != file) {
            return Err(refuse("rebuilds differently from an earlier record of the same file".into()));
        }
        built = Some(file);
    }
    built.ok_or_else(|| FormatError::Ungrab(format!("{name}: no records")))
}

/// The CMAP a sheet is shown with where no entry fixes it: the palette the archive's rules pick for its first image.
/// Note: a viewing aid only; grab never reads those bytes, and ungrab_check.py's first control randomises them.
fn display_cmap(archive: &DatFile, first_image: Option<usize>, rotated: bool) -> Result<[u8; CMAP_LEN]> {
    let mut six = [0u8; CMAP_LEN];
    match first_image {
        Some(n) => {
            let choice = archive.palette_for(n)?;
            six.copy_from_slice(archive.payload_bytes(choice.base)?.get(..CMAP_LEN).ok_or_else(|| FormatError::Ungrab(format!("palette entry {} is short", choice.base)))?);
            if let Some(stamp) = choice.stamp {
                let piece = archive.payload_bytes(stamp.entry)?;
                let at = stamp.first_slot * 3;
                six.get_mut(at..at + piece.len()).ok_or_else(|| FormatError::Ungrab(format!("piece {} runs past the palette", stamp.entry)))?.copy_from_slice(piece);
            }
        }
        // A sheet with no cut (the font, the sampled pixels) is shown under the main palette.
        None => six.copy_from_slice(archive.payload_bytes(MAIN_PALETTE_ENTRY)?.get(..CMAP_LEN).ok_or_else(|| FormatError::Ungrab("the main palette is short".into()))?),
    }
    let mut out = [0u8; CMAP_LEN];
    for (at, slot) in out.iter_mut().enumerate() {
        // A rotated sheet's source colour j is shown as colour rotate_pixel(j) after grab swaps the blocks.
        let colour = if rotated { rotate_pixel((at / 3) as u8) as usize } else { at / 3 };
        *slot = (six[colour * 3 + at % 3] & 0x3f) << 2;
    }
    Ok(out)
}

// --------------------------------------------------------------------------------------------------------------------
// The self-check: cut the container back out of the tree

/// Model the packer's reads over the finished tree and compare with the archive; returns how many entries were checked.
/// Markers and the font are not modelled here; the emulator run in ungrab_check.py covers those.
fn recut(archive: &DatFile, manifest: &Manifest, files: &BTreeMap<String, Vec<u8>>) -> Result<usize> {
    let mut pictures: HashMap<&str, Lbm> = HashMap::new();
    let mut checked = 0usize;
    for (n, op) in manifest.ops.iter().enumerate() {
        let pay = archive.payload_bytes(n)?;
        let file = files.get(op.file()).ok_or_else(|| FormatError::Ungrab(format!("entry {n}: {} is not in the tree", op.file())))?;
        let mismatch = |why: String| FormatError::Ungrab(format!("self-check, entry {n} ({} from {}): {why}", op.how(), op.file()));
        if op.file().ends_with(".lbm") && !pictures.contains_key(op.file()) {
            pictures.insert(op.file(), Lbm::read(file).map_err(|e| mismatch(e.to_string()))?);
        }
        let pixel = |x: usize, y: usize| -> Result<u8> {
            let lbm = pictures.get(op.file()).ok_or_else(|| mismatch("not a picture".into()))?;
            if x >= lbm.width || y >= lbm.height {
                return Err(mismatch(format!("({x},{y}) is outside the {}x{} sheet", lbm.width, lbm.height)));
            }
            Ok(lbm.pixels[y * lbm.width + x])
        };
        match op {
            Op::Cut { rect, load, .. } => {
                let image = IndexedImage::read(pay).map_err(|e| mismatch(e.to_string()))?;
                for y in 0..rect.height() {
                    for x in 0..rect.width() {
                        let got = edit_pixel(pixel(rect.x0 + x + load.x, rect.y0 + y)?, &load.edits);
                        if got != image.pixels[y * rect.width() + x] {
                            return Err(mismatch(format!("pixel ({x},{y}) re-cuts as {got}, the archive has {}", image.pixels[y * rect.width() + x])));
                        }
                    }
                }
            }
            Op::Palette { at, edits, .. } => {
                let lbm = pictures.get(op.file()).ok_or_else(|| mismatch("not a picture".into()))?;
                for (i, want) in pay.iter().enumerate() {
                    let got = lbm.cmap[unedit_cmap(at + i, edits)] >> 2;
                    if got != *want {
                        return Err(mismatch(format!("palette byte {i} reads as {got}, the archive has {want}")));
                    }
                }
            }
            Op::Samples { load, offsets, .. } => {
                for (k, off) in offsets.iter().enumerate() {
                    let got = edit_pixel(pixel(off % SCREEN + load.x, off / SCREEN)?, &load.edits);
                    if got != pay[k] {
                        return Err(mismatch(format!("sampled byte {k} reads as {got}, the archive has {}", pay[k])));
                    }
                }
            }
            Op::Sound { file: name } => {
                let record = audio_record(name, file).map_err(|e| mismatch(e.to_string()))?;
                let same = record.len() == pay.len() && record.iter().zip(pay.iter()).enumerate().all(|(i, (a, b))| AUDIO_UNWRITTEN.contains(&i) || a == b);
                if !same {
                    return Err(mismatch("the record rebuilt from the file differs outside bytes 7 to 10".into()));
                }
            }
            Op::Copy { pad, .. } => {
                let mut want = file.clone();
                if *pad {
                    want.extend_from_slice(&[0u8; DRIVER_PAD]);
                }
                if want != pay {
                    return Err(mismatch("the file plus its pad is not the archive's bytes".into()));
                }
            }
            Op::Font { .. } => continue,
        }
        checked += 1;
    }
    Ok(checked)
}

/// What 3000:c79c makes of an audio file, with the four bytes it never writes left as zero.
pub fn audio_record(name: &str, file: &[u8]) -> Result<Vec<u8>> {
    let (samples, word) = if name.ends_with(".xmi") {
        (file, 0u16)
    } else {
        let header = file.get(..VOC_HEADER).ok_or_else(|| FormatError::malformed(format!("{name}: shorter than a VOC header")))?;
        let length = u32::from_le_bytes([header[27], header[28], header[29], 0]);
        let count = (length.wrapping_sub(2) & 0xffff) as usize;
        let samples = file.get(VOC_HEADER..VOC_HEADER + count).ok_or_else(|| FormatError::malformed(format!("{name}: {count} samples promised, fewer present")))?;
        (samples, voc_rate_word(header[30]))
    };
    if samples.len() > u16::MAX as usize {
        return Err(FormatError::malformed(format!("{name}: {} bytes do not fit the record's size word", samples.len())));
    }
    let mut record = vec![AUDIO_KIND, 0, 0, 0, 0];
    record.extend_from_slice(&(samples.len() as u16).to_le_bytes());
    record.extend_from_slice(&[0, 0, 0, 0]);
    record.extend_from_slice(&word.to_le_bytes());
    record.extend_from_slice(samples);
    Ok(record)
}

// --------------------------------------------------------------------------------------------------------------------
// The notes written beside the sources

fn edits_text(edits: &[Edit]) -> String {
    if edits.is_empty() {
        return "-".into();
    }
    edits.iter().map(|e| match e { Edit::Rotate => "rot".to_string(), Edit::Recolour { from, to } => format!("{from}>{to}") }).collect::<Vec<_>>().join(",")
}

/// One row per STRONG.DAT entry: the file, what grab takes from it, and where, in the file's own coordinates.
fn entry_map(manifest: &Manifest) -> String {
    let mut out = String::from("# Every STRONG.DAT entry and where grab takes it from. Rectangles are inclusive, in the .lbm's own pixels.\n");
    out.push_str("# how: cut, cut+anchor, frame (the 0xfe marker row is y1 + 1), palette (CMAP bytes), font, pixels, audio, copy, copy+15.\n");
    out.push_str("# edits: what grab does to the picture after loading it; 20>0 and 255>0 turn those colours transparent, rot swaps colour blocks.\n");
    out.push_str("entry\tfile\thow\tx0\ty0\tx1\ty1\tedits\tnote\n");
    for (n, op) in manifest.ops.iter().enumerate() {
        let file = op.file().replace('\\', "/");
        let row = match op {
            Op::Cut { rect, load, kind, .. } => {
                let note = if *kind == 4 { format!("marker row {}", rect.y1 + 1) } else { "-".into() };
                format!("{}\t{}\t{}\t{}\t{}\t{}", rect.x0 + load.x, rect.y0, rect.x1 + load.x, rect.y1, edits_text(&load.edits), note)
            }
            Op::Palette { at, size, edits, .. } => format!("-\t-\t-\t-\t{}\tCMAP bytes {} to {} as the game sees them", edits_text(edits), at, at + size - 1),
            Op::Samples { load, offsets, .. } => {
                let at: Vec<String> = offsets.iter().map(|o| format!("{},{}", o % SCREEN + load.x, o / SCREEN)).collect();
                format!("-\t-\t-\t-\t{}\tsingle pixels at {}", edits_text(&load.edits), at.join(" "))
            }
            Op::Font { .. } => "0\t0\t319\t199\t-\tthe whole sheet; 255 delimits glyphs".into(),
            Op::Sound { .. } | Op::Copy { .. } => "-\t-\t-\t-\t-\t-".into(),
        };
        out.push_str(&format!("{n}\t{file}\t{}\t{row}\n", op.how()));
    }
    out
}

fn readme(manifest: &Manifest, recognised: &Recognised, files: usize, pictures: usize, checked: usize, dat: &[u8]) -> String {
    let origin = match recognised {
        Recognised::Shipped => "the shipped English CD STRONG.DAT".to_string(),
        Recognised::ByLayout { md5 } => format!("a STRONG.DAT that is not the shipped one (md5 {md5}), accepted because every entry has the kind and shape grab makes"),
    };
    README_TEMPLATE
        .replace("{origin}", &origin)
        .replace("{dat_md5}", &md5_hex(dat))
        .replace("{exe_md5}", &manifest.exe_md5)
        .replace("{files}", &files.to_string())
        .replace("{pictures}", &pictures.to_string())
        .replace("{checked}", &checked.to_string())
        .replace("{entries}", &manifest.ops.len().to_string())
}

// --------------------------------------------------------------------------------------------------------------------
// Writing it out

impl SourceTree {
    /// Write every file under dir at its DOS path, plus the notes. Refuses a dir that already holds anything.
    pub fn write_to(&self, dir: &Path) -> Result<usize> {
        let io = |path: &Path| { let path = path.to_path_buf(); move |source| FormatError::Io { path, source } };
        if dir.exists() && std::fs::read_dir(dir).map_err(io(dir))?.next().is_some() {
            return Err(FormatError::Ungrab(format!("{} is not empty; ungrab writes only into an empty or new folder", dir.display())));
        }
        std::fs::create_dir_all(dir).map_err(io(dir))?;
        let notes = [(README_NAME.to_string(), self.readme.as_bytes()), (MAP_NAME.to_string(), self.map.as_bytes())];
        let all = self.files.iter().map(|(name, bytes)| (name.clone(), bytes.as_slice())).chain(notes);
        let mut written = 0usize;
        for (name, bytes) in all {
            let path = dos_path(dir, &name)?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(io(parent))?;
            }
            std::fs::write(&path, bytes).map_err(io(&path))?;
            written += 1;
        }
        Ok(written)
    }
}

/// Where a DOS name lands under dir: each backslash a folder, nothing allowed to climb out.
fn dos_path(dir: &Path, name: &str) -> Result<PathBuf> {
    let mut path = dir.to_path_buf();
    for part in name.split('\\') {
        if part.is_empty() || part == "." || part == ".." || part.contains('/') || part.contains(':') {
            return Err(FormatError::Ungrab(format!("{name:?} is not a plain DOS path")));
        }
        path.push(part);
    }
    Ok(path)
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests. The real archive is not in the repo; the tests that need it skip when it is absent.

#[cfg(test)]
mod tests {
    use super::*;

    /// Where the CD build's archive sits under the repo, relative to the repo root.
    const STRONG_DAT_RELATIVE: &str = "achieve/archive.org/en_cdrom/unpacked/STRONG.DAT";

    fn real_archive() -> Option<Vec<u8>> {
        repo_file(STRONG_DAT_RELATIVE)
    }

    /// A file under the repo root, looking from the crate upwards.
    fn repo_file(relative: &str) -> Option<Vec<u8>> {
        let mut dir = std::env::current_dir().ok()?;
        for _ in 0..4 {
            let candidate = dir.join(relative);
            if candidate.is_file() {
                return std::fs::read(candidate).ok();
            }
            if !dir.pop() {
                break;
            }
        }
        None
    }

    #[test]
    fn every_rate_word_in_use_has_a_time_constant_and_vocs_read_back() {
        for tc in [0u8, 100, 131, 165, 210, 255] {
            assert_eq!(voc_rate_word(voc_time_constant(voc_rate_word(tc)).expect("found")), voc_rate_word(tc));
        }
        let mut pay = vec![AUDIO_KIND, 0, 0, 0, 0, 3, 0, 9, 9, 9, 9];
        pay.extend_from_slice(&voc_rate_word(131).to_le_bytes());
        pay.extend_from_slice(&[0x80, 0x81, 0x7f]);
        let voc = rebuild_audio("a.voc", &[(0, &pay)]).expect("rebuilds");
        assert_eq!(&voc[..20], b"Creative Voice File\x1a");
        let record = audio_record("a.voc", &voc).expect("reads back");
        assert_eq!((&record[..7], &record[11..]), (&pay[..7], &pay[11..]));
        let other = { let mut p = pay.clone(); p[13] = 1; p };
        assert!(rebuild_audio("a.voc", &[(0, &pay), (1, &other)]).is_err(), "two records of one file must agree");
        assert!(rebuild_audio("a.xmi", &[(0, &pay)]).is_err(), "an .xmi record carries no rate word");
    }

    #[test]
    fn dos_paths_land_under_the_folder_and_cannot_climb_out() {
        let dir = Path::new("/out");
        assert_eq!(dos_path(dir, "anim\\1cl1f.lbm").expect("plain"), PathBuf::from("/out/anim/1cl1f.lbm"));
        assert!(dos_path(dir, "..\\x").is_err());
        assert!(dos_path(dir, "a\\\\b").is_err());
        assert!(dos_path(dir, "c:x").is_err());
    }

    #[test]
    fn a_container_that_is_not_the_shipped_one_is_refused() {
        let manifest = Manifest::embedded().expect("manifest");
        assert!(recognise(b"not an archive", &manifest, false).is_err(), "unknown md5");
        let as_shipped = Manifest { dat_md5: md5_hex(b"x"), ..manifest.clone() };
        assert!(recognise(b"x", &as_shipped, false).is_err(), "md5 matches, size does not");
        let sized = Manifest { dat_md5: md5_hex(b"x"), dat_size: 1, ..manifest.clone() };
        assert_eq!(recognise(b"x", &sized, false).expect("known"), Recognised::Shipped);
        assert!(matches!(recognise(b"x", &manifest, true), Ok(Recognised::ByLayout { .. })), "let through by layout when asked");
        assert!(ungrab(b"not an archive", true).is_err(), "by layout it still has to be an archive of the right shape");
    }

    #[test]
    fn the_german_floppy_archive_is_refused_even_by_layout() {
        let Some(dat) = repo_file("achieve/archive.org/de_floppy/unpacked/STRONG.DAT") else {
            eprintln!("the German STRONG.DAT is not on this machine; skipping");
            return;
        };
        assert_eq!(md5_hex(&dat), GERMAN_FLOPPY_MD5);
        let err = ungrab(&dat, true).expect_err("refused");
        assert!(err.to_string().contains("German"), "{err}");
    }

    #[test]
    fn the_real_archive_ungrabs_and_re_cuts_exactly() {
        let Some(dat) = real_archive() else {
            eprintln!("STRONG.DAT not on this machine; skipping");
            return;
        };
        assert_eq!(md5_hex(&dat), "96557d434c8960699a125b0fc5e2f96b");
        let tree = ungrab(&dat, false).expect("ungrabs");
        assert_eq!(tree.recognised, Recognised::Shipped);
        assert_eq!((tree.files.len(), tree.pictures), (475, 291));
        assert!(tree.readme.contains("81d23814efc42db5674352469697d654"));
        assert_eq!(tree.map.lines().filter(|l| !l.starts_with('#')).count(), 3452, "a header and 3451 rows");
        for (name, bytes) in &tree.files {
            if name.ends_with(".lbm") {
                let lbm = Lbm::read(bytes).expect("every picture reads back");
                assert!(lbm.width % 2 == 0 && lbm.height == 200, "{name}");
            }
        }
        assert!(tree.files.contains_key("anim\\1cl1f.lbm") && tree.files.contains_key("save.999"));
        let mut edited = dat.clone();
        edited[100] ^= 1;
        assert!(ungrab(&edited, false).is_err(), "one byte changed is not the shipped archive");
    }
}
