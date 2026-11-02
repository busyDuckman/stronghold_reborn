// --------------------------------------------------------------------------------------------------------------------
// One source picture as the constraints the container puts on it, compiled into an LBM the packer reads back.
//
// A transcription of grab_trace.py's Sheet, place_markers, font_sheet and compile_sheet, which the original packer
// has checked by the round trip (about_grab.md). Every pixel an entry fixes is put once; a second, different value
// for the same pixel is an error, never a choice. Anything no entry fixes is colour 0.

use crate::error::{FormatError, Result};
use crate::grab::lbm::{Lbm, CMAP_LEN};
use crate::grab::manifest::{Descriptor, Edit, Rect};
use std::collections::BTreeSet;

/// The frame the packer loads a picture into is 320 by 200.
pub const SCREEN: usize = 320;
pub const ROWS: usize = 200;
/// What 3000:3580 scans a frame's bottom row for to measure its advance.
pub const MARKER: u8 = 0xfe;
/// What 2000:2a25 scans for in a font sheet.
pub const FONT_DELIMITER: u8 = 0xff;
/// Glyphs in a font record.
pub const GLYPHS: usize = 96;

/// Why a pixel holds the value it does, for the message when two reasons disagree.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Why {
    Entry(usize),
    Sample { entry: usize, byte: usize },
    Marker { descriptor: usize, frame: usize },
    Font(usize),
}

/// One frame of an animation sheet as place_markers needs it.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct FrameCut {
    pub rect: Rect,
    /// Byte 5 of the shipped kind 4 record.
    pub advance: u8,
    pub ordinal: usize,
}

/// A picture under construction: fixed pixels, fixed CMAP bytes, and the loads that read it.
pub struct Sheet {
    pub name: String,
    rows: Vec<Vec<Option<(u8, Why)>>>,
    cmap: Vec<Option<(u8, usize)>>,
    max_x: usize,
    /// Every x a cut load placed this picture at.
    pub loads: BTreeSet<usize>,
    /// The ordinal and record of the font entry made from this sheet, if any.
    pub font: Option<(usize, Vec<u8>)>,
    /// True when a load of this sheet swapped the colour blocks, so its CMAP is stored unswapped.
    pub rotated: bool,
}

impl std::fmt::Debug for Sheet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Sheet {} up to x {}", self.name, self.max_x)
    }
}

impl Sheet {
    pub fn new(name: &str) -> Sheet {
        Sheet { name: name.to_string(), rows: vec![Vec::new(); ROWS], cmap: vec![None; CMAP_LEN], max_x: 0, loads: BTreeSet::new(), font: None, rotated: false }
    }

    /// Fix one pixel; a different value already fixed there is an error naming both reasons.
    pub fn put(&mut self, x: usize, y: usize, value: u8, why: Why) -> Result<()> {
        let row = self.rows.get_mut(y).ok_or_else(|| FormatError::Ungrab(format!("{}: {why:?} needs row {y}, below the {ROWS}-row frame", self.name)))?;
        if row.len() <= x {
            row.resize(x + 1, None);
        }
        if let Some((old, old_why)) = row[x] {
            if old != value {
                return Err(FormatError::Ungrab(format!("{} ({x},{y}): {old} for {old_why:?}, {value} for {why:?}", self.name)));
            }
        }
        row[x] = Some((value, why));
        self.max_x = self.max_x.max(x);
        Ok(())
    }

    /// The fixed value of one pixel, if any.
    pub fn get(&self, x: usize, y: usize) -> Option<u8> {
        self.rows.get(y).and_then(|row| row.get(x)).copied().flatten().map(|(v, _)| v)
    }

    /// Fix one CMAP byte (8-bit, as IFF stores it).
    pub fn put_cmap(&mut self, at: usize, value: u8, entry: usize) -> Result<()> {
        let slot = self.cmap.get_mut(at).ok_or_else(|| FormatError::Ungrab(format!("{}: CMAP byte {at} for entry {entry}", self.name)))?;
        if let Some((old, old_entry)) = *slot {
            if old != value {
                return Err(FormatError::Ungrab(format!("{} CMAP byte {at}: {old} for entry {old_entry}, {value} for entry {entry}", self.name)));
            }
        }
        *slot = Some((value, entry));
        Ok(())
    }

    /// Whether a CMAP byte is fixed by an entry, which is what the packer reads of it.
    pub fn cmap_fixed(&self, at: usize) -> bool {
        self.cmap.get(at).is_some_and(|c| c.is_some())
    }

    /// Frame offset to sheet pixel, for a picture loaded at xoff: offsets run on past column 319 into the next row.
    fn at_offset(&self, offset: usize, xoff: usize) -> Option<u8> {
        self.get(offset % SCREEN + xoff, offset / SCREEN)
    }

    /// The sheet's width: 320, or wide enough for every viewport and every fixed pixel, kept even.
    pub fn width(&self) -> usize {
        let viewport = self.loads.iter().any(|x| *x > 0);
        if !viewport && self.max_x < SCREEN {
            return SCREEN;
        }
        let width = (self.max_x + 1).max(self.loads.iter().max().copied().unwrap_or(0) + SCREEN);
        width + (width & 1)
    }

    /// Lay the font out where 2000:2a25 finds it, and fix every pixel of the 320 by 200 sheet to it.
    pub fn place_font(&mut self) -> Result<()> {
        let Some((ordinal, record)) = self.font.clone() else {
            return Ok(());
        };
        let art = font_sheet(&record)?;
        for y in 0..ROWS {
            for x in 0..SCREEN {
                self.put(x, y, art[y * SCREEN + x], Why::Font(ordinal))?;
            }
        }
        Ok(())
    }

    /// Put the 0xfe markers back so 3000:3580 measures the advances the container holds.
    ///   - frame 0's row holds two markers, the origin and its own; every later frame's row holds one.
    ///   - an advance is the previous marker minus this one in the current column, low byte only.
    ///   - a marker may sit at any column with the right residue, so the first layout that fits wins.
    /// Note: a transcription of grab_trace.place_markers, which the packer has checked; keep the two in step.
    pub fn place_markers(&mut self, xoff: usize, descriptors: &[Descriptor], frames: &[FrameCut]) -> Result<()> {
        let mut at = 0usize;
        for (i, desc) in descriptors.iter().enumerate() {
            let mine = frames.get(at..at + desc.frames).ok_or_else(|| FormatError::Ungrab(format!("{}: descriptor {i} wants frames past the {} cut", self.name, frames.len())))?;
            at += desc.frames;
            if !desc.marked {
                continue;
            }
            let plan = self.marker_plan(xoff, desc, mine).ok_or_else(|| FormatError::Ungrab(format!("{}: no marker layout reproduces descriptor {i}'s advances", self.name)))?;
            for (offset, frame) in plan {
                self.put(offset % SCREEN + xoff, offset / SCREEN, MARKER, Why::Marker { descriptor: i, frame })?;
            }
        }
        Ok(())
    }

    /// The first marker layout for one descriptor, as (frame offset, frame index) pairs, or None.
    fn marker_plan(&self, xoff: usize, desc: &Descriptor, mine: &[FrameCut]) -> Option<Vec<(usize, usize)>> {
        let occupied = |off: i64| off >= 0 && self.at_offset(off as usize, xoff).is_some_and(|v| v != MARKER);
        let scan_clear = |start: i64, stop: i64| (start.max(0)..stop).all(|off| self.at_offset(off as usize, xoff) != Some(MARKER));
        let first_frame = mine.first()?;
        let xstep = desc.xstep as i64;
        let total = mine.iter().map(|f| f.advance as i64).sum::<i64>() & 0xff;
        let dist = if total == 0 { 256 } else { total };
        let fx0 = first_frame.rect.x0 as i64;
        for first in desc.x00 as i64..desc.x00 as i64 + 256 {
            let base = (first_frame.rect.y1 as i64 + 1) * SCREEN as i64;
            let own = first + dist;
            if occupied(base + first) || occupied(base + own) || !scan_clear(base + fx0, base + own) {
                continue;
            }
            let mut plan = vec![((base + first) as usize, 0), ((base + own) as usize, 0)];
            let (mut prev, mut column) = (own, 0i64);
            let mut ok = true;
            for (k, frame) in mine.iter().enumerate().skip(1) {
                let x0 = frame.rect.x0 as i64;
                let col = if xstep != 0 { (x0 - fx0).div_euclid(xstep) } else { 0 };
                if col != column {
                    prev += (col - column) * xstep;
                    column = col;
                }
                let want = (prev - frame.advance as i64).rem_euclid(256);
                let base = (frame.rect.y1 as i64 + 1) * SCREEN as i64;
                let mut cur = x0 + (want - x0).rem_euclid(256);
                while cur < x0 + 1024 && (occupied(base + cur) || !scan_clear(base + x0, base + cur)) {
                    cur += 256;
                }
                if cur >= x0 + 1024 {
                    ok = false;
                    break;
                }
                plan.push(((base + cur) as usize, k));
                prev = cur;
            }
            if ok {
                return Some(plan);
            }
        }
        None
    }

    /// The LBM: fixed pixels over colour 0, fixed CMAP bytes over the display palette given.
    /// Note: the display palette is a viewing aid; the packer reads only the fixed bytes, and the controls check that.
    pub fn compile(&self, display: &[u8; CMAP_LEN]) -> Result<Lbm> {
        let width = self.width();
        let mut pixels = vec![0u8; width * ROWS];
        for (y, row) in self.rows.iter().enumerate() {
            for (x, cell) in row.iter().enumerate() {
                if let Some((v, _)) = cell {
                    pixels[y * width + x] = *v;
                }
            }
        }
        let mut cmap = *display;
        for (at, cell) in self.cmap.iter().enumerate() {
            if let Some((v, _)) = cell {
                cmap[at] = *v;
            }
        }
        Ok(Lbm { width, height: ROWS, pixels, cmap })
    }
}

// --------------------------------------------------------------------------------------------------------------------
// The edits, forwards and backwards

/// 3000:bc00 on one pixel: 0x50-0x7f up by 0x50, 0x80-0xcf down by 0x30.
pub fn rotate_pixel(v: u8) -> u8 {
    match v {
        0x50..=0x7f => v + 0x50,
        0x80..=0xcf => v - 0x30,
        _ => v,
    }
}

/// 3000:bc00 on the palette: which old colour new colour i is copied from; also the inverse of rotate_pixel.
pub fn rotate_source(i: u8) -> u8 {
    match i {
        0x50..=0x9f => i + 0x30,
        0xa0..=0xcf => i - 0x50,
        _ => i,
    }
}

/// A source pixel run through the load's edits, as the packer sees it at the cut.
pub fn edit_pixel(mut v: u8, edits: &[Edit]) -> u8 {
    for edit in edits {
        v = match *edit {
            Edit::Rotate => rotate_pixel(v),
            Edit::Recolour { from, to } if v == from => to,
            Edit::Recolour { .. } => v,
        };
    }
    v
}

/// The source pixel the edits turn into v, or None when no source pixel becomes v.
pub fn unedit_pixel(mut v: u8, edits: &[Edit]) -> Option<u8> {
    for edit in edits.iter().rev() {
        match *edit {
            Edit::Rotate => v = rotate_source(v),
            Edit::Recolour { from, to } if v == from && from != to => return None,
            Edit::Recolour { .. } => {}
        }
    }
    Some(v)
}

/// The CMAP byte the edits move to palette byte at.
pub fn unedit_cmap(at: usize, edits: &[Edit]) -> usize {
    let (mut colour, channel) = (at / 3, at % 3);
    for edit in edits.iter().rev() {
        if *edit == Edit::Rotate {
            colour = rotate_source(colour as u8) as usize;
        }
    }
    colour * 3 + channel
}

// --------------------------------------------------------------------------------------------------------------------
// The font

/// A 320 by 200 picture 2000:2a25 parses back into exactly the glyphs of this font record.
///   - the record: height at 0, 96 glyph offsets from 1, 96 widths from 0xc1, pixels from 0x121.
///   - a 0xff in column 0 on row height + 1 gives the height; a delimiter row holds 0xff at each glyph's first and
///     last column, its pixels the height rows below; bands are height + 2 rows apart, band 0 starting at column 1.
pub fn font_sheet(record: &[u8]) -> Result<Vec<u8>> {
    let bad = |why: String| FormatError::Ungrab(format!("font record: {why}"));
    let height = *record.first().ok_or_else(|| bad("empty".into()))? as usize;
    let widths = record.get(0xc1..0xc1 + GLYPHS).ok_or_else(|| bad("too short for its widths".into()))?;
    let mut pix = vec![0u8; SCREEN * ROWS];
    if height + 1 >= ROWS {
        return Err(bad(format!("height {height}")));
    }
    pix[(height + 1) * SCREEN] = FONT_DELIMITER;
    let (mut band, mut col) = (0usize, 1usize);
    for (k, w) in widths.iter().map(|w| *w as usize).enumerate() {
        let offset = u16::from_le_bytes([record[1 + 2 * k], record[2 + 2 * k]]) as usize;
        let glyph = record.get(0x121 + offset..0x121 + offset + w * height).ok_or_else(|| bad(format!("glyph {k} runs past the record")))?;
        if w < 2 {
            return Err(bad(format!("glyph {k} is {w} wide, and a glyph narrower than 2 cannot be delimited")));
        }
        if col + w > SCREEN {
            band += height + 2;
            col = 0;
            if band + height >= ROWS {
                return Err(bad("the glyphs do not fit in 200 rows".into()));
            }
        }
        pix[band * SCREEN + col] = FONT_DELIMITER;
        pix[band * SCREEN + col + w - 1] = FONT_DELIMITER;
        for y in 0..height {
            let at = (band + 1 + y) * SCREEN + col;
            pix[at..at + w].copy_from_slice(&glyph[y * w..(y + 1) * w]);
        }
        col += w;
    }
    Ok(pix)
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_is_undone_exactly() {
        for v in 0..=255u8 {
            assert_eq!(rotate_source(rotate_pixel(v)), v);
            assert_eq!(unedit_pixel(edit_pixel(v, &[Edit::Rotate]), &[Edit::Rotate]), Some(v));
        }
        assert_eq!((rotate_pixel(0x50), rotate_pixel(0x80), rotate_pixel(0xd0)), (0xa0, 0x50, 0xd0));
        assert_eq!(unedit_cmap(0xa0 * 3 + 2, &[Edit::Rotate]), 0x50 * 3 + 2);
    }

    #[test]
    fn a_keyed_colour_has_no_source() {
        let keyed = [Edit::Recolour { from: 20, to: 0 }, Edit::Recolour { from: 255, to: 0 }];
        assert_eq!(unedit_pixel(255, &keyed), None);
        assert_eq!(unedit_pixel(20, &keyed), None);
        assert_eq!(unedit_pixel(0, &keyed), Some(0));
        assert_eq!(edit_pixel(20, &keyed), 0);
    }

    #[test]
    fn two_reasons_for_one_pixel_must_agree() {
        let mut sheet = Sheet::new("a.lbm");
        sheet.put(5, 5, 9, Why::Entry(1)).expect("first");
        sheet.put(5, 5, 9, Why::Entry(2)).expect("same value");
        assert!(sheet.put(5, 5, 8, Why::Entry(3)).is_err());
        assert!(sheet.put(0, ROWS, 1, Why::Entry(4)).is_err(), "below the frame");
        assert_eq!(sheet.width(), SCREEN);
        sheet.put(400, 0, 1, Why::Entry(5)).expect("wide");
        assert_eq!(sheet.width(), 402, "even");
        sheet.loads.insert(220);
        assert_eq!(sheet.width(), 540, "a viewport at 220 needs 320 more");
    }

    #[test]
    fn markers_reproduce_the_advances() {
        // Two frames side by side, advances 3 and 5: frame 0's row gets origin and marker, frame 1 one marker.
        let rect0 = Rect { x0: 0, y0: 0, x1: 9, y1: 9 };
        let rect1 = Rect { x0: 20, y0: 0, x1: 29, y1: 9 };
        let frames = [FrameCut { rect: rect0, advance: 3, ordinal: 0 }, FrameCut { rect: rect1, advance: 5, ordinal: 1 }];
        let mut sheet = Sheet::new("anim.lbm");
        sheet.put(0, 10, 7, Why::Entry(9)).expect("an obstacle on the marker row");
        sheet.place_markers(0, &[Descriptor { x00: 0, xstep: 0, frames: 2, marked: true }], &frames).expect("places");
        let marks: Vec<usize> = (0..SCREEN).filter(|x| sheet.get(*x, 10) == Some(MARKER)).collect();
        // The origin skips the obstacle at 0; frame 0's marker is 8 on; frame 1's is 5 before it, mod 256.
        assert_eq!(marks, vec![1, 9, 260]);
        assert_eq!((9 - 260i64).rem_euclid(256), 5);
    }

    #[test]
    fn a_font_record_lays_out_in_bands() {
        let height = 3usize;
        let mut record = vec![height as u8];
        let mut pixels = Vec::new();
        for k in 0..GLYPHS {
            record.extend_from_slice(&((k * 4 * height) as u16).to_le_bytes());
            pixels.extend(std::iter::repeat_n((k + 1) as u8, 4 * height));
        }
        record.extend(std::iter::repeat_n(4u8, GLYPHS));
        record.extend_from_slice(&pixels);
        let art = font_sheet(&record).expect("lays out");
        assert_eq!(art[(height + 1) * SCREEN], FONT_DELIMITER, "the height marker");
        assert_eq!((art[1], art[4], art[SCREEN + 1]), (FONT_DELIMITER, FONT_DELIMITER, 1));
        // 79 glyphs of 4 fit after column 1; the 80th starts band 1 at column 0.
        assert_eq!(art[(height + 2) * SCREEN], FONT_DELIMITER);
        assert_eq!(art[(height + 3) * SCREEN], 80);
        record[0xc1] = 1;
        assert!(font_sheet(&record).is_err(), "a glyph 1 wide cannot be delimited");
    }
}
