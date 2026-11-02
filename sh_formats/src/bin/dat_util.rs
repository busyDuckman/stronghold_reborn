// --------------------------------------------------------------------------------------------------------------------
// dat_util: poke at STRONG.DAT from a terminal. A test tool for the sh_formats crate, not the game.
//
//   dat_util list                          every entry: index, offset, size, type, dimensions, name
//   dat_util info 36 hourgls4.lbm:3        everything known about some entries, palette rule included
//   dat_util extract build\geninn01.lbm    raw payloads to .output/dat_util/, one file per entry
//   dat_util extract --all                 the whole archive
//   dat_util export 1170-1199 --filter xbr2x   images as PNG, sampled sounds as WAV, music as MID
//   dat_util export 1170-1199 --strip      a run of unit frames as one animation strip
//   dat_util names                         every source file and the entries it became
// Entries are named by index, an inclusive range (36-44), a source file (every entry cut from
// it), or one cut from it (hourgls4.lbm:3).

use clap::{Parser, Subcommand};
use sh_formats::prelude::*;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Where the CD build's archive sits under the repo, relative to the repo root.
const STRONG_DAT_RELATIVE: &str = "achieve/archive.org/en_cdrom/unpacked/STRONG.DAT";
/// Where extracted payloads go unless told otherwise, relative to the repo root.
const DEFAULT_OUT: &str = ".output/dat_util";

#[derive(Parser)]
#[command(name = "dat_util", about = "Extract and inspect entries of STRONG.DAT", version)]
struct Cli {
    /// The archive. Defaults to the CD build under achieve/, searched for from the cwd upwards.
    #[arg(long, global = true)]
    dat: Option<PathBuf>,
    /// A names file; repeatable, later files add to earlier ones. Defaults to the .tsv beside the archive.
    #[arg(long, global = true)]
    names: Vec<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Every entry, one line each.
    List {
        /// Only entries with this type label (image1, image4, pal, pcm, xmi, txt, ...).
        #[arg(long = "type")]
        only_type: Option<String>,
    },
    /// Everything known about the given entries.
    Info { refs: Vec<String> },
    /// Write the raw payload of each entry to a file.
    Extract {
        refs: Vec<String>,
        /// Every entry in the archive.
        #[arg(long)]
        all: bool,
        /// Output folder.
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Write images as PNG under their palette and sampled sounds as WAV.
    Export {
        refs: Vec<String>,
        /// Reprocess images first: xbr2x, or nearest:N for N by N blocks.
        #[arg(long)]
        filter: Option<String>,
        /// Treat the entries as one animation and write a single strip.
        #[arg(long)]
        strip: bool,
        /// Output folder.
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Every source file the names file binds, and the entries it became.
    Names,
    /// The palette entries and which images each is chosen for.
    Palettes,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("dat_util: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let dat_path = match cli.dat {
        Some(path) => path,
        None => find_upwards(STRONG_DAT_RELATIVE).ok_or_else(|| FormatError::malformed(format!("no --dat given and {STRONG_DAT_RELATIVE} is not under the cwd or its parents")))?,
    };
    let dat = if cli.names.is_empty() {
        DatFile::open(&dat_path)?
    } else {
        let mut names = NameTable::empty();
        for path in &cli.names {
            names.extend(NameTable::load(path)?);
        }
        DatFile::open_with_names(&dat_path, names)?
    };
    match cli.command {
        Command::List { only_type } => list(&dat, only_type.as_deref()),
        Command::Info { refs } => {
            for index in entries_for_args(&dat, &refs)? {
                info(&dat, index)?;
            }
            Ok(())
        }
        Command::Extract { refs, all, out } => {
            let indices = if all { (0..dat.entry_count()).collect() } else { entries_for_args(&dat, &refs)? };
            if indices.is_empty() {
                return Err(FormatError::malformed("nothing to extract: give entry references or --all"));
            }
            extract(&dat, &indices, &output_dir(out))
        }
        Command::Export { refs, filter, strip, out } => {
            let indices = entries_for_args(&dat, &refs)?;
            if indices.is_empty() {
                return Err(FormatError::malformed("nothing to export: give entry references"));
            }
            let filter = filter.as_deref().map(parse_filter).transpose()?;
            let out = output_dir(out);
            if strip { export_strip(&dat, &indices, filter.as_deref(), &out) } else { export(&dat, &indices, filter.as_deref(), &out) }
        }
        Command::Names => {
            for source in dat.names().source_files() {
                println!("{:>5} {:>5} {:>4}  {:<10} {}", source.first_entry, source.last_entry, source.entry_count(), format!("{:?}", source.confidence).to_lowercase(), source.name);
            }
            println!("{} source files", dat.names().len());
            Ok(())
        }
        Command::Palettes => palettes(&dat),
    }
}

/// The folder given, else .output/dat_util under the repo root, else relative to the cwd.
fn output_dir(given: Option<PathBuf>) -> PathBuf {
    given.or_else(|| find_upwards(".output").map(|p| p.join("dat_util"))).unwrap_or_else(|| PathBuf::from(DEFAULT_OUT))
}

/// The --filter spellings: xbr2x, nearest:N.
fn parse_filter(text: &str) -> Result<Box<dyn ImageFilter>> {
    match text.split_once(':') {
        None if text == "xbr2x" => Ok(Box::new(Xbr2x::default())),
        Some(("nearest", n)) => n.parse::<usize>().map(|factor| Box::new(NearestScale { factor }) as Box<dyn ImageFilter>).map_err(|_| FormatError::malformed(format!("nearest:{n} is not a whole factor"))),
        _ => Err(FormatError::malformed(format!("{text:?} is not a filter; try xbr2x or nearest:3"))),
    }
}

/// Walk from the cwd towards the root looking for a relative path.
fn find_upwards(relative: &str) -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let candidate = dir.join(relative);
        if candidate.exists() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// One argument: 36, 36-44 inclusive, a source file (every entry cut from it), or x.lbm:3 (its fourth cut).
fn entries_for_arg(dat: &DatFile, text: &str) -> Result<Vec<usize>> {
    let text = text.trim();
    if let Ok(index) = text.parse::<usize>() {
        return Ok(vec![index]);
    }
    if let Some((first, last)) = text.split_once('-').and_then(|(a, b)| Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?))) {
        if first > last {
            return Err(FormatError::malformed(format!("range {text} runs backwards")));
        }
        return Ok((first..=last).collect());
    }
    // A DOS name never has a colon after the first character, so a trailing :n is a position.
    if let Some((name, nth)) = text.rsplit_once(':').filter(|(name, _)| !name.is_empty()) {
        let nth = nth.parse::<usize>().map_err(|_| FormatError::malformed(format!("{nth:?} in {text} is not a position")))?;
        return Ok(vec![dat.entry_named(name, nth)?]);
    }
    dat.entries_named(text)
}

fn entries_for_args(dat: &DatFile, refs: &[String]) -> Result<Vec<usize>> {
    let mut indices = Vec::new();
    for text in refs {
        for index in entries_for_arg(dat, text)? {
            dat.entry(index)?;
            indices.push(index);
        }
    }
    Ok(indices)
}

/// What the type label leaves out: an image's size and anchor, a sound's rate.
fn describe_type(file_type: &SourceFileType) -> String {
    match file_type {
        SourceFileType::Image(h) => format!("{}x{} anchor ({},{}) advance {}", h.width, h.height, h.anchor_x, h.anchor_y, h.advance),
        SourceFileType::Audio(h) => format!("{} Hz, {} bytes", h.sample_rate, h.data_size),
        _ => String::new(),
    }
}

fn name_column(dat: &DatFile, index: usize) -> String {
    match dat.origin_of(index) {
        Some(p) => format!("{}:{}", p.source.name, p.nth),
        None => String::new(),
    }
}

fn list(dat: &DatFile, only_type: Option<&str>) -> Result<()> {
    let mut shown = 0;
    for entry in dat.entries() {
        let file_type = dat.source_type(entry.index)?;
        if only_type.is_some_and(|t| t != file_type.label()) {
            continue;
        }
        println!("{:>5} {:>8} {:>7}  {:<8} {:<36} {}", entry.index, entry.offset, entry.size, file_type.label(), describe_type(file_type), name_column(dat, entry.index));
        shown += 1;
    }
    println!("{shown} of {} entries", dat.entry_count());
    Ok(())
}

fn info(dat: &DatFile, index: usize) -> Result<()> {
    let entry = dat.entry(index)?;
    let file_type = dat.source_type(index)?;
    println!("entry {index}: offset {} size {} type {} {}", entry.offset, entry.size, file_type.label(), describe_type(file_type));
    match dat.origin_of(index) {
        Some(p) => println!("  from: {}, sprite {} of {} ({:?})", p.source.name, p.nth, p.source.entry_count(), p.source.confidence),
        None => println!("  from: no source file known"),
    }
    if file_type.is_image() {
        let choice = dat.palette_for(index)?;
        let stamp = choice.stamp.map(|s| format!(" with entry {} stamped at slot {}", s.entry, s.first_slot)).unwrap_or_default();
        println!("  palette: entry {}{stamp}", choice.base);
        println!("  rule: {:?}, {}", choice.rule, choice.rule.describe());
        let image = dat.read_image(index)?;
        let written = image.mask.iter().filter(|m| **m).count();
        println!("  pixels: {} of {} written", written, image.pixels.len());
    }
    if is_music(file_type) {
        let xmi: XmiScore = dat.read_as(index)?;
        let score = dat.score(index)?;
        let timbres: Vec<String> = xmi.sequences[0].timbres.iter().map(|t| format!("{}:{}", t.bank, t.patch)).collect();
        println!("  music: {} sequences, {} events, {:.1}s", xmi.sequences.len(), xmi.sequences[0].events.len(), score.duration_seconds());
        println!("  score: {} messages, {} ticks at {} a quarter", score.len(), score.duration_ticks(), score.ticks_per_quarter());
        println!("  channels: {:?}", score.channels());
        println!("  timbres: {}", if timbres.is_empty() { "none named".to_string() } else { timbres.join(" ") });
    }
    Ok(())
}

/// The file an extracted entry is written to: index, then the source file and position if known.
fn output_filename(dat: &DatFile, index: usize) -> String {
    let ext = dat.source_types()[index].extension();
    match dat.origin_of(index) {
        Some(p) => {
            let stem = Path::new(&p.source.name.replace('\\', "_")).with_extension("");
            format!("{index:04}_{}_{}.{ext}", stem.display(), p.nth)
        }
        None => format!("{index:04}.{ext}"),
    }
}

fn extract(dat: &DatFile, indices: &[usize], out: &Path) -> Result<()> {
    std::fs::create_dir_all(out).map_err(|source| FormatError::Io { path: out.to_path_buf(), source })?;
    for index in indices {
        let path = out.join(output_filename(dat, *index));
        std::fs::write(&path, dat.payload_bytes(*index)?).map_err(|source| FormatError::Io { path: path.clone(), source })?;
        println!("{}", path.display());
    }
    println!("{} entries written to {}", indices.len(), out.display());
    Ok(())
}

/// The file an exported entry is written to: the extraction name with the filter and format on it.
fn export_filename(dat: &DatFile, index: usize, filter: Option<&dyn ImageFilter>, extension: &str) -> String {
    let stem = Path::new(&output_filename(dat, index)).with_extension("");
    match filter {
        Some(f) => format!("{}_{}.{extension}", stem.display(), f.name()),
        None => format!("{}.{extension}", stem.display()),
    }
}

/// XMI, whether the packer wrapped it in a kind 3 header or took the IFF whole.
fn is_music(file_type: &SourceFileType) -> bool {
    matches!(file_type, SourceFileType::Audio(h) if h.audio_type == AudioType::Xmi) || *file_type == SourceFileType::Xmi
}

/// Each image as a PNG, each sampled sound as a WAV, each score as a MID; the rest is named and skipped.
fn export(dat: &DatFile, indices: &[usize], filter: Option<&dyn ImageFilter>, out: &Path) -> Result<()> {
    std::fs::create_dir_all(out).map_err(|source| FormatError::Io { path: out.to_path_buf(), source })?;
    let mut written = 0;
    for index in indices {
        let file_type = dat.source_type(*index)?;
        let path = if file_type.is_image() {
            let mut image = dat.render_image(*index)?;
            if let Some(f) = filter {
                image = f.apply(&image);
            }
            let path = out.join(export_filename(dat, *index, filter, Png::EXTENSION));
            export_to_file(&Png, &image, &path)?;
            path
        } else if matches!(file_type, SourceFileType::Audio(h) if h.audio_type == AudioType::Pcm) {
            let path = out.join(export_filename(dat, *index, None, Wav::EXTENSION));
            export_to_file(&Wav, &dat.audio_clip(*index)?, &path)?;
            path
        } else if is_music(file_type) {
            let path = out.join(export_filename(dat, *index, None, Midi::EXTENSION));
            export_to_file(&Midi, &dat.score(*index)?, &path)?;
            path
        } else {
            println!("entry {index} is {}, nothing to export it as", file_type.label());
            continue;
        };
        println!("{}", path.display());
        written += 1;
    }
    println!("{written} files written to {}", out.display());
    Ok(())
}

/// The entries as one animation, side by side on a common baseline, as one PNG.
fn export_strip(dat: &DatFile, indices: &[usize], filter: Option<&dyn ImageFilter>, out: &Path) -> Result<()> {
    std::fs::create_dir_all(out).map_err(|source| FormatError::Io { path: out.to_path_buf(), source })?;
    let (first, last) = (indices[0], indices[indices.len() - 1]);
    if indices.len() != last - first + 1 {
        return Err(FormatError::malformed("a strip needs one consecutive run of entries"));
    }
    let mut strip = dat.animation(first, indices.len())?.strip();
    if let Some(f) = filter {
        strip = f.apply(&strip);
    }
    let name = format!("{first:04}-{last:04}_strip");
    let path = out.join(match filter {
        Some(f) => format!("{name}_{}.png", f.name()),
        None => format!("{name}.png"),
    });
    export_to_file(&Png, &strip, &path)?;
    println!("{} ({}x{})", path.display(), strip.width(), strip.height());
    Ok(())
}

fn palettes(dat: &DatFile) -> Result<()> {
    let entries = dat.palette_indices();
    for pal in &entries {
        let mut by_rule = std::collections::BTreeMap::new();
        for index in 0..dat.entry_count() {
            if let Ok(choice) = dat.palette_for(index) {
                if choice.base == *pal {
                    *by_rule.entry(format!("{:?}", choice.rule)).or_insert(0usize) += 1;
                }
            }
        }
        let uses: Vec<String> = by_rule.iter().map(|(rule, n)| format!("{n} {}", rule.to_lowercase())).collect();
        println!("{:>5}  {:<28} {}", pal, name_column(dat, *pal), uses.join(", "));
    }
    println!("{} palettes", entries.len());
    Ok(())
}
