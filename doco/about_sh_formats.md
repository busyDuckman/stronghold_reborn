# sh_formats: readers for the game's own file formats

The `sh_formats/` crate reads STRONG.DAT and what is inside it. It is a library first; the
`dat_util` bin is a test tool. The format knowledge is a port of what the reversing pass
established and every claim below cites where in `reversing/notes/` it comes from. Nothing in
this crate invents a fact the notes do not have.

## The shape

Three layers. The file formats are what is in the archive; the archive is where they live;
media is what a modern consumer takes away, with nothing of the game's encoding left on it.

```
sh_formats/src/
  file_format.rs           trait FileFormat { read(bytes), source_type() }, enum SourceFileType
  file_type_detector.rs    detect_file_type(bytes): which format a payload is
  error.rs                 FormatError, the one error type
  formats/                 one handler per payload format, each a FileFormat
    image.rs               IndexedImage: the plain image and the Placed and Frame records round it
    palette.rs             Palette (768 bytes) and PalettePiece (144), 6-bit widened to 8
    audio.rs               AudioHeader: the 13-byte wrapper and what is behind it
    music.rs               XmiScore: the AIL/Miles IFF behind the wrapper, and its events
  dat/                     the archive
    dat_file.rs            DatFile: the entry table, every entry's type, names, palette rules,
                           and the bridge to media (render_image, sprite, animation, audio_clip)
    name_table.rs          NameTable: which art file each entry was cut from, and how sure that is
    palette_rules.rs       PaletteChoice, PaletteRule, the PaletteResolver trait, KnownPaletteRules
  media/                   for modern use; knows nothing about the archive
    image.rs               Image: an RGBA raster, and the Png that writes one out
    sprite.rs              Sprite (image + anchor), Frame, Animation, SpriteSet<S: SpriteState>
    audio.rs               AudioClip: samples at a rate, and the Wav that writes one out
    music.rs               Score: MIDI messages on one timeline, Message, and the Midi that writes one out
    export.rs              trait Exporter<T> and export_to_file, and nothing else
    filter.rs              traits ImageFilter, AudioFilter; NearestScale, Xbr2x, Resample
    xbr.rs                 the xBRjs 2x kernel, line for line
  grab/                    the packer's side: STRONG.DAT back into the files STRONG.EXE grab reads
    manifest.rs            Manifest, Op: what grab makes of each file, traced; grab_manifest.tsv embedded
    sheet.rs               Sheet: one picture's constraints, the 0xfe markers, the font layout
    lbm.rs                 Lbm: the IFF PBM the game's reader accepts, written and read
    md5.rs                 which STRONG.DAT this is
    ungrab.rs              ungrab(dat) -> SourceTree, its self-check, and writing it out
  bin/dat_util.rs          the CLI
  bin/ungrab.rs            the ungrab CLI, a thin shell over grab::ungrab
```

`FileFormat::read` never guesses: bytes that are not the format come back as an error saying
why. `detect_file_type` tries the readers in `strongdat.py`'s order and names what the packer took
whole (drivers, text, FLI, the save) by signature. `DatFile` is the one object a caller holds.

The traits, and why each exists:

| trait | implemented by | why a trait |
|---|---|---|
| `FileFormat` | every handler in `formats/` | the detector and `DatFile::read_as` are written against it |
| `PaletteResolver` | `KnownPaletteRules`, `DatFile` | the rules below now; the decompile's own answer later |
| `Exporter<T>` | `Png` for `Image`, `Wav` for `AudioClip`, `Midi` for `Score` | one destination format per type, more to come |
| `ImageFilter`, `AudioFilter` | `NearestScale`, `Xbr2x`; `Resample` | a filter is a value with settings, keepable in a list or a menu |
| `SpriteState` | `BuildingState`, `UnitState` | a `SpriteSet` is keyed by whatever the thing's states are |

An exporter lives beside the type it writes rather than in `export.rs`, so a type and
everything that can be done to it are in one file. `export.rs` holds the trait and
`export_to_file` and knows about no format at all.

## Opening it

```rust
use sh_formats::prelude::*;
let dat = DatFile::open(Path::new("STRONG.DAT"))?;               // names from STRONG.tsv beside it, if present
let dat = DatFile::open_with_names(path, NameTable::load(tsv)?)?;  // names from wherever
let dat = DatFile::from_bytes(bytes, path, NameTable::empty())?;
```

`DatFile::open` is the opinionated one: the names file is the `.dat`'s own name with a `.tsv`
extension. A missing names file is logged and tolerated, because the port ships without one;
every lookup by name then misses and every entry is still reachable by index.

The entry table (the notes call it the index table) is checked, not trusted: payloads must
run contiguously from offset 4 to the table and the table must divide into 21-byte rows
(`about_assets.md`, "The container"). A file that fails is refused with the check named. Opening classifies every entry once; the CD
build's 3451 take well under a second.

## Three words

- An **entry** is one payload in STRONG.DAT, numbered from 0. Entry 36 is one sprite.
- A **source file** is an art sheet the packer was fed, such as `build\geninn01.lbm`. Not
  STRONG.DAT: one of the 361 `.lbm` files that went into it. Those files no longer exist; only
  their names survive, in the executable.
- One source file was cut into several sprites, which became **consecutive entries**. The
  sprites of `build\geninn01.lbm` are entries 36 to 44, so entry 40 is "the fifth sprite cut
  from `build\geninn01.lbm`". (`about_sprite_names.md`, which calls the span a run.)

## Access by index or by name

```rust
dat.payload_bytes(36)?                     // the payload as stored
dat.read_image(36)?                        // decoded, or an error saying why 36 is not an image
dat.entries_named("build\\geninn01.lbm")?  // every entry cut from that art file: 36..=44
dat.entry_named("hourgls4.lbm", 3)?        // the fourth sprite cut from that file
dat.origin_of(40)                          // which art file 40 was cut from, and which cut it was
```

Forward slashes and any case are accepted; the packer's spelling is DOS with backslashes.
A name can occur twice (`csite1.lbm` does) and then names both spans.

## The names file

Tab separated, `#` comments, two row shapes, so the reversing pass's own files load as they
are and can be concatenated:

    name  first  last  [length  grade  evidence]    one source file, as reversing/work/names/segmentation.tsv
    index  name                                      one entry by hand, as reversing/notes/dat_names.tsv

The names were recovered from the executable and not every binding is certain, so each
`SourceFile` carries a `Confidence`: `Confirmed` (the executable names the file and the
archive's sprites are the sizes it cut), `Candidate` (a positional guess that drifts),
`Unstated` (no grade column), or `User` (a row of `dat_names.tsv`, which the reversing pass
treats as settled: today the two FLIs, paired by decoding the frames and looking). A `User`
row wins over any source file covering the same index.

`segmentation.tsv` is derived output under the untracked `reversing/work/`, regenerated by
`just dat-segment` from `reversing/`. Copy it beside the `.dat` as `STRONG.tsv`, or pass it
with `--names`, which the CLI accepts more than once.

## What an entry is

`detect_file_type` gives the same counts on the CD build as the python: 3192 images (134 plain, 868
placed, 2190 frames), 56 palettes, 9 palette pieces, 138 audio, 34 text, 2 FLI. There is no
type field (`about_assets.md`, "The trap"): the leading byte nominates a layout and a
payload is an image only once its header and pixel stream account for its length exactly.
`SourceFileType::Image` and `::Audio` carry their parsed headers, so a listing never decodes
twice.

Images decode with a written mask beside the pixels. Colour 0 is never written and that
absence is the transparency; a decoder that writes the zeros looks right on a single sprite
and punches black holes when composited. Keep the mask.

## Which palette

`PaletteResolver::palette_for` returns a `PaletteChoice`: the base palette entry, an optional
piece to stamp over it, and the `PaletteRule` that chose it. The rule travels with the answer
so a default can never be mistaken for a fact. `DatFile::palette(&choice)` builds the
`Palette`; `palette_for_image` does both. The rules, in order, each with its source:

1. **Adjacent.** The palette stored beside the image. Portraits (115x104, and only those)
   carry theirs after: `3000:39fb` cuts 0,0 to 114,103 and stashes 768 bytes after it.
   Credits carry theirs before: `6000:e846` stashes the palette, then cuts. The portrait
   size is what tells the two apart and keeps the frame after a portrait's palette from
   claiming it. Where the names file places both entries they must be from the same file.
2. **Backdrop.** An image beside a 144-byte piece: the main palette with the piece stamped
   at slots 160..=207. From the owner's notes on entry 944 and the check recorded there. Sets
   are image, piece, image; the first half's piece follows it and the second half's precedes.
3. **SameSource.** A palette stashed out of the same `.lbm` as the image
   (`about_sprite_names.md`: talkwin9.lbm is 902 to 906). Needs a names file.
4. **Inspected.** The owner's readings by eye in `manual_inspection_of_palettes.md` and the
   asset browser notes: the dialog and credits scenes where several files share one palette
   (918 to 922 under 917, and so on). A fixed table in `palette_rules.rs`.
5. **Default.** The main game palette, entry 741 by the owner's inspection, for everything
   else, which is every unit and building. This is a default, not a claim. Which palette a
   unit really uses comes out of the decompile and is still open.

`KnownPaletteRules::for_cd_build` hard-codes 741; `KnownPaletteRules::new` takes another
default for an archive that is not the CD build, and refuses one that is not a palette.

## Media: what comes out

`DatFile` bridges to `media` with `render_image(index)` (an `Image` under the palette the
rules choose), `sprite(index)` (the image plus the anchor its record carries), `animation(first,
count)` (a run of `Frame` records, each with its advance), `audio_clip(index)` (bare PCM only:
the 8SVX payloads are not decoded yet) and `score(index)` (the music).

```rust
let walk = dat.animation(1170, 30)?;                 // the spearman
let strip = Xbr2x::default().apply(&walk.strip());   // every frame on a common baseline, doubled
export_to_file(&Png, &strip, Path::new("spearman.png"))?;
let sound = Resample { sample_rate: 44100 }.apply(&dat.audio_clip(1025)?);
export_to_file(&Wav, &sound, Path::new("1025.wav"))?;
export_to_file(&Midi, &dat.score(1017)?, Path::new("1017.mid"))?;   // the long one, 110.6 seconds
```

What the sprite types encode is what the reversing pass has established and no more:

- A `Sprite` is a picture and its **anchor**, the point that lands on the position it is
  drawn at (`about_assets.md`: a Placed record draws at `(x - ax, y - ay)`). Units anchor at
  the feet, buildings hang off their footprint tile.
- A `Frame` carries its **advance**, how far the unit moves on that frame; an `Animation` is
  a run of them, and its `advance_per_loop` is why playing it faster moves the unit faster.
- The original stores one facing and mirrors at draw time, so **facing is an operation**
  (`Sprite::mirrored`, `Animation::mirrored`), not a stored state.
- A `SpriteSet<S>` maps a state to an animation. `BuildingState { view, upgrade, damaged }`
  is what `dat_sprites.yaml` records. `UnitState { level, sequence }` is as far as the code
  goes: a creature has a table of frame sequences and which one is "walk" or "attack" is not
  recovered, so a sequence is a number until the decompile names it.

`Xbr2x` is a port of the 2x kernel of xBRjs, the library the asset browser draws with, and
its test holds the port to that library's output byte for byte (`test_data/xbr2x/`,
regenerated by `scripts/gen_xbr_fixture.py`). Ported rather than written from the algorithm
because the vendored file's provenance note records an in-house kernel being thrown out for
wrong blending.

## Music: XMI to MIDI

The 43 music entries are AIL/Miles XMI behind the same kind 3 header the sampled sounds use,
which is why their sample rate field reads zero. `XmiScore` reads the IFF with or without
that header in front of it, so `read_as` works on an entry either way.

Every one in the CD build has the same shape: `FORM XDIR` with a two byte `INFO` count, then
a `CAT XMID` holding a single `FORM XMID` of an optional `TIMB` and a mandatory `EVNT`. An
unknown chunk is refused rather than skipped, the way `strongfli.py` refuses an unknown FLI
chunk: a chunk quietly passed over is a plausible wrong answer.

XMI is a MIDI dialect with two differences, and `Score::from_xmi` is those two differences:

- **Notes state their own length.** There are no note off events; a note on carries how long
  it sounds as a variable length quantity after the velocity, so every one schedules its own
  off. A note on of zero velocity is already a note off and is left alone.
- **An interval is 1/120 second of real time, fixed.** The tempo event in the stream is the
  composer's notation and the AIL driver does not time against it. The AIL manual states the
  rate in the rules for the For and Next loop controllers, which require "more than 1/120
  second (8.4 milliseconds) of real time" between them, that being one quantization interval.

Timing is the part a converter usually gets wrong, and both ways of getting it right are not
equal. Dropping the tempo event and writing 120 BPM plays correctly and loses the composer's
tempo, so nothing lines up with a bar in an editor. Instead the tempo is kept and the score's
ticks are worked out against the tempo in force, at 960 to the quarter note: the file says 100
BPM in 4/4 where the composer wrote that, and it still plays for exactly as long as the game
played it. Ticks are measured from the last tempo change rather than accumulated gap by gap,
so rounding cannot drift over a song.

The end of track marker belongs to the file format and not to the music, so `Score` never
holds one and `Midi` writes it. `Score` is format 0, one track, which is what XMI already is:
the AIL manual notes that a format 1 file is merged during XMI creation.

What the archive does not contain is any sampled instrument. Entry 1109, `midpak.ad`, is the
OPL2 timbre bank the `TIMB` chunks index: 256 directory records of patch, bank and offset,
then 256 fourteen byte records of a length prefix and twelve bytes of OPL register data, bank
0 melodic and bank 127 percussion. That is the game's own FM instrument set and it is not
decoded here. Playing a converted score on a sampled synthesiser means bringing a soundfont.

None of the 43 uses an AIL controller in the 110 to 119 range, so there are no loop points or
branches to carry across, which is the one thing that usually does not survive the conversion.

The whole corpus is checked at once rather than by ear: `every_score_in_the_real_archive_converts`
converts all 43, holds each to the length its intervals say, requires every note that starts
to be stopped (4592 of them), and writes each as a MIDI file.

## Ungrab: the packer's side

`STRONG.EXE grab` is the packer that built STRONG.DAT, still in the executable
(`reversing/notes/about_grab.md`). `grab::ungrab` runs it backwards: STRONG.DAT in, the source
tree grab reads out, every file at the name and folder grab opens it by (`anim\`, `build\`,
`portrait\` as subfolders). An enthusiast edits the art in a paint program and rebuilds the
container by running the original `STRONG grab` in DOSBox.

    just ungrab .output/ungrab                       the shipped STRONG.DAT
    just ungrab .output/mod path/to/STRONG.DAT --accept-layout

The output holds the 475 files grab opens (291 pictures, 130 audio files, 34 texts, two FLIs,
`save.999` and 17 sound drivers) plus `ungrab.txt`, the modder's instructions (how to rebuild,
which STRONG.EXE, the reserved colours, the 64K chunk limit, what is never read), and
`ungrab.tsv`, every entry's rectangle in the picture's own pixels. Both are 8.3 names grab never
opens. The folder must be new or empty.

**What it knows, and why a manifest.** Every entry's file, rectangle, kind, load position and
edits, the animation descriptors, the eight sampled offsets: all of it is in STRONG.EXE, but it
only comes out by running the packer, which takes the emulator, the far heap seams and the DOS
file layer of `grab_trace.py`. Reading it from STRONG.EXE at run time would mean porting that
machine or recovering the packer, which the port does not ship. So it is carried as
`sh_formats/src/grab/grab_manifest.tsv` (3,451 entry rows, 79 descriptor rows, 165 KB),
embedded with `include_str!`. It is generated from a trace, never written by hand:
`reversing/scripts/ungrab_check.py --write-manifest` regenerates it, and a plain run of that
script fails if the embedded copy and a fresh trace differ by a byte. The data in it is the
packer's structure, not the art: pixels, palettes, marker positions, the font layout and the
VOC headers are all computed from STRONG.DAT at run time.

**Which STRONG.DAT.** The English CD build (md5 `96557d434c8960699a125b0fc5e2f96b`), which the
manifest was traced against. Anything else is refused with its md5 named. The German floppy
STRONG.DAT (`f7a424df...`) is refused even with `--accept-layout`: it has 3,451 entries too, but
its STRONG.EXE is a different build whose packer was never traced, so a tree laid out for the
English packer is not known to be what the German grab reads. `--accept-layout` exists for the
modder's own rebuilt container: an unknown md5 is accepted when every entry still has the kind
and shape the packer makes (image dimensions, palette and sample lengths, the audio header), and
every rebuild check below still refuses.

**How each entry goes back.** A transcription of `grab_trace.py`'s rebuild, which the original
packer has checked:

- a cut's pixels go back at its rectangle plus the load's x (the 17 viewport sheets are wider
  than 320 and stored ByteRun1 to stay under the 64K chunk limit), with the load's edits undone:
  `3000:bc00`'s colour block swap reversed, and a colour the keyed loads turn to 0 refused;
- two entries that want different values for one pixel are an error, never a choice;
- palette entries go into the CMAP (shifted left two), unswapped for the rotated sheets;
- the `0xfe` markers are placed by the same search as `place_markers`, so `3000:3580` measures
  each frame's advance; the font is laid out the way `2000:2a25` parses it; the eight pixels of
  `triangls.lbm` go back at the offsets read out of the code;
- audio records become VOC files whose block 1 length and time constant give the record's
  sample count and rate word, `.xmi` records the payload; all records of one file must agree;
- the drivers lose the 15 zero bytes grab appends; texts, FLIs and `save.999` are copied.

Before the tree is returned, `recut` reads every written picture back and cuts each entry out
again the way the packer does (load x, edits, rectangle; CMAP slice; sampled offsets; the audio
record; the copy and its pad), comparing with the archive: 3,450 of 3,451 entries, all but the
font. A tree that does not re-cut is never written. The font and the markers are not modelled
there; the emulator run proves them.

**The CMAP.** grab reads a picture's palette only where it stashes it (56 whole palettes, and
144 bytes of `l1` to `l9`). The rest of every CMAP is never read, so ungrab fills it with the
palette `KnownPaletteRules` picks, with no names file, for the sheet's first image (rotated where
the sheet is), or the main palette for a sheet with no image, so a
paint program shows the art in roughly the right colours instead of black. That is a viewing
aid and, for many sheets, a reading rather than a fact; the proof randomises those bytes and
nothing changes.

**The LBMs** are FORM PBM, 8 planes, a 20-byte BMHD, a 768-byte CMAP and a BODY stored at 320
wide or ByteRun1 when wider, every chunk under 64K and every width even, which is what
`2000:3801` dictates (planar ILBM is accepted by the game but its path has never been run).
Within that, PBM is Deluxe Paint's own chunky format. Checked by the proof wherever the tools are
installed (they were for this note): netpbm's `ilbmtoppm` and ffmpeg's IFF decoder read all 291
pictures to exactly the pixels and colours.
**Not checked**: GIMP, Deluxe Paint and GrafX2, none of which is on the machine this was built on
and none of which could be installed without the owner's scan. Whether GIMP opens PBM at all is
unverified.

**The proof**, `reversing/scripts/ungrab_check.py` (about two minutes): the original packer,
under the emulator, over the Rust tree alone gives back STRONG.DAT with 0 bytes different outside
bytes 7 to 10 of the 138 kind 3 audio records. Those 552 bytes are heap the packer never writes;
under the harness's zero fill 435 of them differ, they are listed in `heap_bytes.tsv`, and runs
under three heap fills move exactly those 552 and nothing else. The tree also agrees with the
Python rebuild file for file (pixels and every CMAP byte grab reads), phase 1's three controls
come out as they must on it, and mirroring one spearman frame (entry 1170) in
`anim\1fi1m.lbm` changes exactly that entry's payload, which decodes to the mirrored pixels;
ungrabbing that edited container with `--accept-layout` and running grab again gives it back.

What it cannot recover and does not need: the developers' art outside the rectangles, most
sheets' palettes, the sheet widths they used, the VOC header fields grab does not read. The tree
is one valid source for the same STRONG.DAT, not the originals. Nothing after `5000:e477`
returns has been run, and nothing here has been run in DOSBox.

## The CLI

    just dat-util list [--type pal]         every entry: index, offset, size, type, dimensions, name
    just dat-util info 36 hourgls4.lbm:3    everything known, palette rule included
    just dat-util extract build\\geninn01.lbm 3438      raw payloads to .output/dat_util/
    just dat-util extract --all
    just dat-util export 1170-1199 --filter xbr2x   images as PNG, sounds as WAV, music as MID
    just dat-util export 1170-1199 --strip  a run of unit frames as one animation strip
    just dat-util names                     every source file and the entries it became
    just dat-util palettes                  each palette and how many images each rule sends to it

Entry arguments are an index, an inclusive range (`36-44`), a source file (every entry cut
from it) or one cut from it (`hourgls4.lbm:3`). Extraction writes the payload bytes as they are in the
archive, named `<index>_<source>_<nth>.<ext>` where the source is known and `<index>.<ext>`
otherwise. `export` writes PNGs under the palette the rules choose, WAVs of the sampled
sounds and MIDs of the music, through `--filter xbr2x` or `--filter nearest:N` if asked. The
filters are picture and sound ones; there is nothing worth filtering on a score yet, and
rendering one to audio needs a synthesiser and the OPL2 bank rather than a filter.
`info` on a music entry prints its sequences, events, length, channels and timbres.

Without `--dat` the CLI looks for the CD build under `achieve/` from the cwd upwards. That
file is not in the repo and is not ours to redistribute; the tests that need it skip when it
is absent and say so.

## Where the design came from

- `reversing/notes/about_assets.md` and `about_loader.md`: the archive, the records, the
  codec, the audio header.
- `reversing/notes/about_sprite_names.md`: a name binds to a span of entries, and how sure each binding is.
- `reversing/notes/manual_inspection_of_palettes.md` and the browser notes on 944: the
  palette rules.
- `landscape_shader/src/dat.rs`: the first Rust decoder, which this crate supersedes. Its
  `stamped` and the backdrop slot came from there.
- `reversing/scripts/strongdat.py` and `datweb.py`: the classifier and the adjacency rule.
- `reversing/notes/about_grab.md` and `reversing/scripts/grab_trace.py`: the packer's trace,
  the rebuild `grab::ungrab` transcribes, and the manifest it carries.
- The AIL/Miles XMIDI specification, as carried in `wildmidi/docs/formats/XMIFileFormat-AIL.txt`:
  the chunk layout, the controller assignments and the 1/120 second interval.
