// --------------------------------------------------------------------------------------------------------------------
// Which of the two conventions a shader is written in.
//
// paired:  foo.glsl + foo.vert, same stem, both watched. GLSL 330, flat `uniform` declarations,
//          one fragment output, compiled and drawn by macroquad. The original convention, and
//          still the right one for a 2D effect or a post pass.
// unified: one file carrying both stages behind #ifdef guards. GLSL 450 as naga reads it:
//          uniform blocks with explicit bindings, several fragment outputs, a G-buffer worth
//          of PBR channels. Compiled by naga and drawn offscreen by wgpu.
//
// The file says which it is, because a flag you have to remember is a flag you will get
// wrong. Detection is by the things only one convention can have; an explicit
// `// !(dialect=...)` on the #version line overrides it where a file is genuinely ambiguous.

use crate::glsl_meta::hint_tokens;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect { Paired, Unified }

impl Dialect {
    pub fn name(self) -> &'static str {
        match self {
            Dialect::Paired => "paired",
            Dialect::Unified => "unified",
        }
    }
}

/// Why a file was read as the dialect it was read as. Printed by --check, because "it picked
/// the wrong one" is otherwise a long afternoon.
#[derive(Clone, Debug)]
pub struct Detection {
    pub dialect: Dialect,
    pub because: String,
    pub errors: Vec<String>,
}

/// The `#version` number, if the file declares one.
pub fn version_of(src: &str) -> Option<u32> {
    for line in src.lines() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix("#version") else { continue };
        return rest.trim_start().split_whitespace().next().and_then(|n| n.parse().ok());
    }
    None
}

/// An explicit `// !(dialect=paired|unified)`, wherever in the file it sits.
/// Anything else in that block is left for the other readers; only dialect= is taken here.
fn explicit_dialect(src: &str) -> (Option<Dialect>, Vec<String>) {
    let mut errs = Vec::new();
    for line in src.lines() {
        let Some(idx) = line.find("//") else { continue };
        for tok in hint_tokens(&line[idx..]).unwrap_or_default() {
            if tok.key != "dialect" { continue; }
            match tok.value.to_ascii_lowercase().as_str() {
                "paired"  => return (Some(Dialect::Paired), errs),
                "unified" => return (Some(Dialect::Unified), errs),
                other => errs.push(format!("dialect '{}' is not paired or unified", other)),
            }
        }
    }
    (None, errs)
}

/// True if the line is an `#ifdef NAME` / `#if defined(NAME)` guard for this define.
pub fn guards_define(line: &str, define: &str) -> bool {
    let t = line.trim_start();
    if let Some(rest) = t.strip_prefix("#ifdef") {
        if !rest.starts_with(char::is_whitespace) { return false; }
        return rest.trim_start().split_whitespace().next() == Some(define);
    }
    if let Some(rest) = t.strip_prefix("#if ") {
        let squashed: String = rest.chars().filter(|c| !c.is_whitespace()).collect();
        return squashed == format!("defined({})", define);
    }
    false
}

/// Which convention this source is written in, and why.
pub fn detect(src: &str) -> Detection {
    let (explicit, errors) = explicit_dialect(src);
    if let Some(d) = explicit {
        return Detection { dialect: d, because: "declared by !(dialect=) in the file".to_string(), errors };
    }

    // A uniform block cannot be compiled by the paired path at all, so it settles the question.
    if src.lines().any(|l| crate::unified::parse::is_block_open(l)) {
        return Detection {
            dialect: Dialect::Unified,
            because: "a layout(...) uniform block".to_string(),
            errors,
        };
    }
    // Two stages in one file is the other thing only the unified path can do.
    for stage_define in ["VERTEX", "FRAGMENT"] {
        if src.lines().any(|l| guards_define(l, stage_define)) {
            return Detection {
                dialect: Dialect::Unified,
                because: format!("an #ifdef {} stage guard", stage_define),
                errors,
            };
        }
    }
    if let Some(v) = version_of(src) {
        if v >= 400 {
            return Detection { dialect: Dialect::Unified, because: format!("#version {}", v), errors };
        }
    }
    Detection {
        dialect: Dialect::Paired,
        because: "no uniform block, stage guard or #version 400+".to_string(),
        errors,
    }
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_330_fragment_shader_is_paired() {
        let src = "#version 330\nuniform float u_a;\nvoid main() {}\n";
        let d = detect(src);
        assert_eq!(d.dialect, Dialect::Paired, "{}", d.because);
    }

    #[test]
    fn a_uniform_block_is_unified_whatever_the_version_says() {
        let src = "#version 330\nlayout(std140, binding = 0) uniform G {\n float a;\n} g;\n";
        assert_eq!(detect(src).dialect, Dialect::Unified);
    }

    #[test]
    fn a_stage_guard_is_unified() {
        let src = "#version 450\n#ifdef VERTEX\nvoid main() {}\n#endif\n";
        let d = detect(src);
        assert_eq!(d.dialect, Dialect::Unified);
        assert!(d.because.contains("VERTEX"), "{}", d.because);
    }

    #[test]
    fn version_450_alone_is_enough() {
        let src = "#version 450\nvoid main() {}\n";
        let d = detect(src);
        assert_eq!(d.dialect, Dialect::Unified);
        assert!(d.because.contains("450"), "{}", d.because);
    }

    #[test]
    fn an_explicit_declaration_wins() {
        let src = "#version 450   // !(dialect=paired)\n#ifdef VERTEX\n#endif\n";
        assert_eq!(detect(src).dialect, Dialect::Paired);

        let src = "#version 330 // !(dialect=nonsense)\nvoid main(){}\n";
        let d = detect(src);
        assert_eq!(d.dialect, Dialect::Paired);
        assert!(d.errors.iter().any(|e| e.contains("not paired or unified")), "{:?}", d.errors);
    }

    #[test]
    fn guards_recognise_both_spellings() {
        assert!(guards_define("#ifdef FRAGMENT", "FRAGMENT"));
        assert!(guards_define("  #if defined( FRAGMENT )", "FRAGMENT"));
        assert!(!guards_define("#ifdef FRAGMENTS", "FRAGMENT"));
        assert!(!guards_define("#ifdefFRAGMENT", "FRAGMENT"));
    }

    #[test]
    fn the_version_line_is_found_past_leading_comments() {
        assert_eq!(version_of("// a note\n#version 450 core\n"), Some(450));
        assert_eq!(version_of("void main() {}\n"), None);
    }
}
