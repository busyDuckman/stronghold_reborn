UNGRAB: the files STRONG.EXE grab builds STRONG.DAT from
==========================================================

Written by ungrab (the sh_formats crate) from {origin},
md5 {dat_md5}. There are {files} files here, every one STRONG.EXE grab opens, each at the
name and folder grab opens it by; {pictures} of them are pictures (.lbm). Before writing
them, ungrab cut {checked} of the {entries} entries back out of these files and compared them
with STRONG.DAT; the font and the animation markers are proved by running grab itself (below).

This file (ungrab.txt) and the entry map (ungrab.tsv) are notes for you. grab never opens them.


Rebuilding STRONG.DAT
---------------------
1. Use the English CD STRONG.EXE, md5 {exe_md5}. These files were laid out for its packer;
   another build's grab may want other files or other rectangles.
2. Work in a COPY of the game folder. The first thing grab does is truncate the STRONG.DAT in
   the current folder, before it has read a single source file.
3. Copy everything in this folder into that copy, keeping the subfolders (anim, build,
   portrait and the rest) as they are.
4. In DOSBox, in that folder, type:   STRONG grab
5. A missing or unreadable file stops it with "File read error <name>" and the file's slot
   number. When it finishes it prints "getcache 3451  SIZESTASH 3475", waits for a key, and
   goes on into the game with the new STRONG.DAT.

The new STRONG.DAT is the shipped one, byte for byte, if nothing was edited, except bytes 7
to 10 of each of the 138 audio records: grab never writes those four bytes, so they hold
whatever was in memory. No source file can set them and the game does not read them.


Editing the pictures
--------------------
- Keep each picture a 256-colour IFF PBM (Deluxe Paint's own chunky "LBM"), 8 bit planes,
  the same width and height, and an even width. Do not save planar ILBM: the game's reader
  has an ILBM path, but no one has ever run it.
- Every IFF chunk must be under 65,536 bytes; the game's reader cannot size a larger one. A
  320 x 200 picture fits uncompressed. The wider sheets (viewports into a scene) must be saved
  compressed (ByteRun1), as they are here.
- Only what ungrab.tsv lists is read: a rectangle per sprite, in this file's own pixels. The
  rectangles are fixed in STRONG.EXE. Paint inside them; anything outside is never read, and
  art that spills over an edge is cut off.
- Colour 0 is transparent. The game's sprite format never writes it.
- Reserved colours:
  * 254 on the row just below each animation frame (the map gives that row) is a marker:
    the distance between markers is how far the unit moves on that frame. Do not move or
    delete one, and do not paint 254 on that row between the frame's left edge and its
    marker, or grab will stop at your pixel instead.
  * Where the map's edits column says 20>0 or 255>0, grab turns those colours into colour 0
    right after loading the picture, so they come out transparent.
  * demfont1.lbm is the font. 255 in column 0 marks the glyph height, and 255 at each
    glyph's first and last column on the top row of its band marks the glyph. Keep that
    layout; a glyph cannot be narrower than 2 pixels.
  * triangls.lbm is read as eight single pixels (listed in the map), not a rectangle.
- Where the edits column says rot (the portraits, l1 to l9 and a few others), grab swaps
  colours 80-127 with 128-207 after loading, pixels and palette together. The file holds
  them unswapped; just paint with the palette in the file and it comes out right.


The palette in each file
------------------------
grab reads a picture's palette only where it stores it in STRONG.DAT: the map's palette rows,
whole palettes, or for l1 to l9 the CMAP bytes it names. Editing those bytes changes that
palette in the game. Every other palette byte is never read. ungrab fills those with the
palette the game is believed to draw the picture with, so the art looks right while you work;
for many sheets that is a reading of the game, not a fact, and changing it changes nothing.


Sounds, music and the rest
--------------------------
- .voc: grab reads 32 bytes of header (block 1's length and time constant) and then the
  samples. Keep a plain 8-bit mono VOC with the sound in one block, under 65,536 samples.
- .xmi: copied in whole after a 13-byte header.
- .txt, .fli, save.999 and midpak.ad: copied as they are.
- The .com and .adv sound drivers: grab adds 15 zero bytes after each, so they are stored
  here without them.


What is not here
----------------
The developers' pictures held more than the rectangles; the rest never went into STRONG.DAT
and cannot be recovered. These files are one valid source for the same STRONG.DAT, not the
originals. About ninety names in STRONG.EXE's file tables are never opened by grab and are not
written; grab does not need them.
