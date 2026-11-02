// --------------------------------------------------------------------------------------------------------------------
// Which art file each entry came from. The archive records no names; they come from the
// executable and from the owner, via a names file kept beside the .dat.
//
// Three words, from about_sprite_names.md:
//   - an entry is one payload in STRONG.DAT, numbered from 0. Entry 36 is one sprite.
//   - a source file is an art sheet the packer was fed, e.g. build\geninn01.lbm. Those files
//     no longer exist; only their names survive, in the executable.
//   - one source file was cut into several sprites, which became consecutive entries: the
//     sprites of build\geninn01.lbm are entries 36 to 44. So a name binds to a span of
//     entries, and an entry is "the nth sprite cut from that file".
// The names were recovered from the executable by the reversing pass and not all of the
// bindings are certain, so each carries a Confidence and the caller can see how much to trust it.
//
// The names file is tab separated, comments start with #, and two row shapes are accepted so
// the reversing pass's own outputs load unchanged:
//   name  first  last  [length  grade  evidence]     one source file, as reversing/work/names/segmentation.tsv
//   index  name                                       one entry named by hand, as dat_names.tsv
// The grade column is the confidence: confirmed, candidate, or blank. A dat_names.tsv row is
// User and wins over any source file covering the same index.

use crate::error::{FormatError, Result};
use std::path::Path;

/// How sure we are that a name really belongs to its entries. Ordered least to most.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Confidence {
    /// A guess from position alone, the kind that is right for a while and then drifts.
    Candidate,
    /// The names file did not say.
    Unstated,
    /// The executable names the file and the archive's sprites are the sizes it cut.
    Confirmed,
    /// A row of dat_names.tsv, settled by looking at the entry. Wins over everything.
    User,
}

impl Confidence {
    fn parse(text: &str) -> Option<Confidence> {
        match text.trim().to_ascii_lowercase().as_str() {
            "confirmed" => Some(Confidence::Confirmed),
            "candidate" => Some(Confidence::Candidate),
            "user" => Some(Confidence::User),
            "" => Some(Confidence::Unstated),
            _ => None,
        }
    }
}

/// One art file the packer was fed, and the span of consecutive entries its sprites became.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SourceFile {
    /// As the packer spelled it, e.g. build\geninn01.lbm.
    pub name: String,
    /// The first entry cut from this file.
    pub first_entry: usize,
    /// The last entry cut from this file, inclusive.
    pub last_entry: usize,
    pub confidence: Confidence,
}

impl SourceFile {
    /// How many entries were cut from this file; never zero.
    pub fn entry_count(&self) -> usize {
        self.last_entry - self.first_entry + 1
    }

    pub fn contains(&self, index: usize) -> bool {
        (self.first_entry..=self.last_entry).contains(&index)
    }

    /// The archive index of the nth sprite cut from this file, from 0.
    pub fn entry(&self, nth: usize) -> Result<usize> {
        if nth >= self.entry_count() {
            return Err(FormatError::NoSuchFrame { name: self.name.clone(), len: self.entry_count(), nth });
        }
        Ok(self.first_entry + nth)
    }
}

/// Where an entry came from: the art file it was cut out of, and that it was the nth cut.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EntryOrigin<'a> {
    pub source: &'a SourceFile,
    /// The entry was the nth sprite cut from the file, from 0.
    pub nth: usize,
}

/// Anything that can say which art file an entry came from, and which entries a file became.
pub trait NameLookup {
    /// Every source file with this name, in entry order. A name can occur twice (csite1.lbm does).
    fn source_files_named(&self, name: &str) -> Vec<&SourceFile>;

    /// Which art file an entry was cut from, and which cut it was. If two rows claim the entry, the more confident wins.
    fn origin_of(&self, index: usize) -> Option<EntryOrigin<'_>>;

    /// Every source file, in entry order.
    fn source_files(&self) -> &[SourceFile];

    /// The first source file with this name, or UnknownName.
    fn source_file_named(&self, name: &str) -> Result<&SourceFile> {
        let found = self.source_files_named(name);
        match found.len() {
            0 => Err(FormatError::UnknownName(name.to_string())),
            _ => Ok(found[0]),
        }
    }
}

/// The names file, loaded: every source file it binds, sorted by first entry.
#[derive(Clone, Default, Debug)]
pub struct NameTable {
    sources: Vec<SourceFile>,
}

/// The packer's spelling is case-insensitive DOS with backslashes; accept either slash.
pub fn normalise_name(name: &str) -> String {
    name.trim().to_ascii_lowercase().replace('/', "\\")
}

impl NameTable {
    /// No names at all. Every lookup misses and every origin_of is None.
    pub fn empty() -> NameTable {
        NameTable::default()
    }

    /// Load a names file from disk.
    pub fn load(path: &Path) -> Result<NameTable> {
        let text = std::fs::read_to_string(path).map_err(|source| FormatError::Io { path: path.to_path_buf(), source })?;
        NameTable::parse(&text)
    }

    /// Parse names file text in either row shape described at the top of this file.
    pub fn parse(text: &str) -> Result<NameTable> {
        let mut sources = Vec::new();
        for (line_no, line) in text.lines().enumerate() {
            let line = line.trim_end();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let cols: Vec<&str> = line.split('\t').collect();
            let fail = |reason: String| FormatError::Names { line: line_no + 1, reason };
            // The header row of a segmentation file, or of a hand-written one.
            if matches!(cols[0], "name" | "index") {
                continue;
            }
            let source = if let Ok(index) = cols[0].trim().parse::<usize>() {
                let name = cols.get(1).map(|c| c.trim()).filter(|c| !c.is_empty()).ok_or_else(|| fail("an index with no name".into()))?;
                SourceFile { name: name.to_string(), first_entry: index, last_entry: index, confidence: Confidence::User }
            } else {
                if cols.len() < 3 {
                    return Err(fail(format!("expected name, first, last; got {} columns", cols.len())));
                }
                let first = cols[1].trim().parse::<usize>().map_err(|_| fail(format!("first entry {:?} is not a number", cols[1])))?;
                let last = cols[2].trim().parse::<usize>().map_err(|_| fail(format!("last entry {:?} is not a number", cols[2])))?;
                if last < first {
                    return Err(fail(format!("run {first}..{last} runs backwards")));
                }
                let confidence_text = cols.get(4).copied().unwrap_or("");
                let confidence = Confidence::parse(confidence_text).ok_or_else(|| fail(format!("unknown confidence {confidence_text:?}, expected confirmed, candidate or user")))?;
                SourceFile { name: cols[0].trim().to_string(), first_entry: first, last_entry: last, confidence }
            };
            sources.push(source);
        }
        sources.sort_by_key(|s| (s.first_entry, s.last_entry));
        Ok(NameTable { sources })
    }

    /// Add another table's source files to this one, so a hand-written file can sit over an inferred one.
    pub fn extend(&mut self, other: NameTable) {
        self.sources.extend(other.sources);
        self.sources.sort_by_key(|s| (s.first_entry, s.last_entry));
    }

    pub fn len(&self) -> usize {
        self.sources.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }
}

impl NameLookup for NameTable {
    fn source_files_named(&self, name: &str) -> Vec<&SourceFile> {
        let wanted = normalise_name(name);
        self.sources.iter().filter(|s| normalise_name(&s.name) == wanted).collect()
    }

    fn origin_of(&self, index: usize) -> Option<EntryOrigin<'_>> {
        // A few hundred source files, so a scan is fine. The most confident claim wins, User beating everything.
        self.sources.iter().filter(|s| s.contains(index)).max_by_key(|s| s.confidence).map(|source| EntryOrigin { source, nth: index - source.first_entry })
    }

    fn source_files(&self) -> &[SourceFile] {
        &self.sources
    }
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# comment\n\
name\tfirst\tlast\tlength\tgrade\tevidence\n\
mouse03.lbm\t0\t3\t4\tconfirmed\tgrab path\n\
build\\geninn01.lbm\t36\t44\t9\tconfirmed\tgrab path\n\
csite1.lbm\t757\t760\t4\tcandidate\tplaced by structure\n\
csite1.lbm\t800\t801\t2\tcandidate\tplaced by structure\n\
3438\tstrong.fli\n\
36\tinn_bottom_left.lbm\n";

    #[test]
    fn both_row_shapes_load_and_names_resolve_either_slash() {
        let names = NameTable::parse(SAMPLE).expect("parses");
        assert_eq!(names.len(), 6);
        let source = names.source_file_named("BUILD/GENINN01.LBM").expect("found");
        assert_eq!((source.first_entry, source.last_entry, source.confidence), (36, 44, Confidence::Confirmed));
        assert_eq!(source.entry(8).expect("last"), 44);
        assert!(source.entry(9).is_err());
        assert_eq!(names.source_files_named("csite1.lbm").len(), 2);
        assert!(names.source_file_named("nothing.lbm").is_err());
        let fli = names.source_file_named("strong.fli").expect("user name");
        assert_eq!((fli.first_entry, fli.last_entry, fli.confidence), (3438, 3438, Confidence::User));
    }

    #[test]
    fn origin_prefers_the_hand_written_name() {
        let names = NameTable::parse(SAMPLE).expect("parses");
        let p = names.origin_of(40).expect("in the inn run");
        assert_eq!((p.source.name.as_str(), p.nth), ("build\\geninn01.lbm", 4));
        let p = names.origin_of(36).expect("named twice");
        assert_eq!((p.source.name.as_str(), p.source.confidence), ("inn_bottom_left.lbm", Confidence::User));
        assert!(names.origin_of(5000).is_none());
        let mut merged = NameTable::parse("x.lbm\t100\t101\n").expect("parses");
        merged.extend(names);
        assert_eq!(merged.len(), 7);
        assert_eq!(merged.source_files()[0].first_entry, 0, "sorted after the merge");
        assert!(NameTable::empty().origin_of(0).is_none());
    }

    #[test]
    fn bad_rows_are_refused_with_a_line_number() {
        assert!(matches!(NameTable::parse("x.lbm\t5\t2\n"), Err(FormatError::Names { line: 1, .. })));
        assert!(matches!(NameTable::parse("\n\nx.lbm\ta\tb\n"), Err(FormatError::Names { line: 3, .. })));
        assert!(NameTable::parse("x.lbm\t1\t2\t2\tguess\n").is_err());
        assert!(NameTable::parse("7\t\n").is_err());
        assert!(NameTable::parse("x.lbm\t1\t2\n").expect("no confidence column").source_files()[0].confidence == Confidence::Unstated);
    }
}
