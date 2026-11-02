// --------------------------------------------------------------------------------------------------------------------
// Tune regions: the unified convention's answer to the paired convention's #else consts.
//
// A unified shader's uniform blocks are filled by the host, so the values that ship with the
// shader live in the code as plain assignments: a default table the host copies, or what the
// shader falls back to when it is compiled with its defaults define. Those assignments are
// the shader's real defaults, so they are what the editor loads and what "Save tuning" writes.
//
//   Terrain default_terrain(int t) {
//       Terrain k;
//       switch (t) {
//           case 0:   // !(tune=terrain.terrain[0]) grass
//               k.cover_order = 2.0;
//               k.bump = 0.35;
//               break;
//
// A region opens on the line carrying !(tune=<path>) and runs to the next such marker, or to
// the first following `break;` or `return`, whichever comes first. Inside it, every
// `<anything>.<field> = <value>;` line is the value of `<path>.<field>`.
//
// Save rewrites only the text between the '=' and the ';', so alignment and trailing comments
// survive, exactly as the paired path's const writer does. An assignment naming a member no
// block has is an error: the host's own compile is what is at stake and it is not tested here.

use std::sync::LazyLock;

use regex::Regex;

use crate::glsl_meta::{matches_fmt, GLSLType};
use crate::tuning::{fmt_glsl_value, parse_glsl_value, ConstVal};
use crate::unified::parse::{split_comment, Unified};
use crate::values::UniformValues;

/// One `k.field = value;` line inside a tune region.
#[derive(Clone, Debug)]
pub struct TuneAssign {
    pub key: String,      // the block-member path this sets, eg "terrain.terrain[0].bump"
    pub raw: String,      // the value text as written
    pub line: usize,      // 0-based line index; Save rewrites in place
}

static ASSIGN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*[A-Za-z_]\w*\s*\.\s*([A-Za-z_]\w*(?:\s*\[\s*\d+\s*\])?)\s*=\s*([^;]+);").expect("literal regex")
});

/// The `tune=` path on a line, if it carries one.
fn tune_marker(line: &str) -> Option<String> {
    let (_, comment) = split_comment(line);
    for tok in crate::glsl_meta::hint_tokens(comment).unwrap_or_default() {
        if tok.key == "tune" {
            let path = tok.value.trim().to_string();
            return if path.is_empty() { None } else { Some(path) };
        }
    }
    None
}

/// True where a tune region stops: the shapes that end a switch arm or a function.
fn ends_region(code: &str) -> bool {
    let t = code.trim_start();
    t.starts_with("break;") || t.starts_with("return")
}

/// Every tune assignment in the file, in file order.
pub fn parse_tune(src: &str) -> (Vec<TuneAssign>, Vec<String>) {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = Vec::new();
    let mut errs = Vec::new();
    let mut path: Option<String> = None;

    for (i, line) in lines.iter().enumerate() {
        let (code, _) = split_comment(line);
        if let Some(p) = tune_marker(line) {
            if p.contains(' ') {
                errs.push(format!("line {}: tune path '{}' has a space in it", i + 1, p));
            }
            path = Some(p);
        } else if path.is_some() && ends_region(code) {
            path = None;
        }
        let Some(prefix) = path.as_deref() else { continue };
        let Some(cap) = ASSIGN.captures(code) else { continue };
        let field: String = cap[1].chars().filter(|c| !c.is_whitespace()).collect();
        out.push(TuneAssign {
            key: format!("{}.{}", prefix, field),
            raw: cap[2].trim().to_string(),
            line: i,
        });
    }
    (out, errs)
}

/// Check the assignments against the blocks, and load them as the starting values.
/// or_insert on purpose: a tune value beats the default= hint, which is seeded after this,
/// but a live slider tweak survives a reload.
pub fn seed_tune_values(values: &mut UniformValues, assigns: &[TuneAssign], shader: &Unified) -> Vec<String> {
    let mut errs = Vec::new();
    for a in assigns {
        let Some((_, leaf)) = shader.find_leaf(&a.key) else {
            errs.push(format!("tune {} (line {}): no block member has that path", a.key, a.line + 1));
            continue;
        };
        match parse_glsl_value(leaf.ty, &a.raw) {
            Some(ConstVal::F(v))  => { values.float.entry(a.key.clone()).or_insert(v); }
            Some(ConstVal::V2(v)) => { values.vec2.entry(a.key.clone()).or_insert(v); }
            Some(ConstVal::V3(v)) => { values.vec3.entry(a.key.clone()).or_insert(v); }
            Some(ConstVal::V4(v)) => { values.vec4.entry(a.key.clone()).or_insert(v); }
            Some(ConstVal::I(v))  => { values.int.entry(a.key.clone()).or_insert(v); }
            Some(ConstVal::U(v))  => { values.uint.entry(a.key.clone()).or_insert(v); }
            None => errs.push(format!("tune {} (line {}): cannot read '{}' as a {}",
                a.key, a.line + 1, a.raw, crate::glsl_meta::ty_name(leaf.ty))),
        }
    }
    errs
}

/// Which block members are tuned, so the controls can show them as the ones Save commits.
pub fn tuned_keys(assigns: &[TuneAssign]) -> Vec<String> {
    assigns.iter().map(|a| a.key.clone()).collect()
}

/// Write the live values onto the tune assignments. Only the text between the '=' and the ';'
/// of a matched line changes, so alignment and trailing comments survive. Line endings too.
/// Pure text transform, so it is unit tested rather than tested by looking at a file.
pub fn write_tune_values(src: &str, assigns: &[TuneAssign], shader: &Unified, values: &UniformValues)
    -> Result<(String, usize), String>
{
    if assigns.is_empty() {
        return Err("no !(tune=...) region in this shader".to_string());
    }
    let crlf = src.contains("\r\n");
    let trailing_nl = src.ends_with('\n');
    let mut lines: Vec<String> = src.lines().map(str::to_string).collect();

    let mut saved = 0usize;
    for a in assigns {
        let Some((_, leaf)) = shader.find_leaf(&a.key) else { continue };
        if matches!(leaf.ty, GLSLType::Sampler2D) { continue; }
        // a member the editor binds from the bake size is not the editor's to write back: the
        // number in the file is the host's business, and the slider never held it
        if matches_fmt(&leaf.hint, "resolution") || matches_fmt(&leaf.hint, "texel") { continue; }
        let Some(value) = fmt_glsl_value(leaf.ty, &a.key, values) else { continue };
        let Some(line) = lines.get(a.line).cloned() else { continue };
        let Some(eq) = line.find('=') else { continue };
        let Some(semi) = line[eq..].find(';').map(|s| eq + s) else { continue };
        lines[a.line] = format!("{}= {}{}", &line[..eq], value, &line[semi..]);
        saved += 1;
    }

    let eol = if crlf { "\r\n" } else { "\n" };
    let mut out = lines.join(eol);
    if trailing_nl { out.push_str(eol); }
    Ok((out, saved))
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unified::parse::parse;

    const SRC: &str = "\
#version 450
struct Terrain {
    vec3 colour;     // !(format=RGB, default=#808080)
    float bump;      // !(default=0.5, min=0, max=1)
};
layout(std140, binding = 0) uniform Globals {
    float patch_frequency;   // !(default=3, min=1, max=12)
    float pixel_step;        // !(format=texel)
} globals;
layout(std140, binding = 1) uniform TerrainBlock {
    Terrain terrain[2];      // !(names=[grass, rock])
} mats;
#ifdef FRAGMENT
layout(location = 0) out vec4 o_albedo;   // !(pbr=albedo:rgb)

Globals default_globals() {
    Globals g;                  // !(tune=globals)
    g.patch_frequency = 3.0;    // patches across the tile
    g.pixel_step = 0.001;
    return g;
}

Terrain default_terrain(int t) {
    Terrain k;
    switch (t) {
        case 0:   // !(tune=mats.terrain[0]) grass
            k.colour = vec3(0.33, 0.52, 0.18);
            k.bump   = 0.35;    // keep this comment
            break;
        default:  // !(tune=mats.terrain[1]) rock
            k.colour = vec3(0.45, 0.43, 0.4);
            k.bump   = 1.0;
            break;
    }
    return k;
}
void main() {}
#endif
";

    #[test]
    fn regions_pick_up_their_assignments_and_stop_at_the_break() {
        let (assigns, errs) = parse_tune(SRC);
        assert!(errs.is_empty(), "{errs:?}");
        let keys: Vec<&str> = assigns.iter().map(|a| a.key.as_str()).collect();
        assert_eq!(keys, vec![
            "globals.patch_frequency", "globals.pixel_step",
            "mats.terrain[0].colour", "mats.terrain[0].bump",
            "mats.terrain[1].colour", "mats.terrain[1].bump",
        ]);
        assert_eq!(assigns[2].raw, "vec3(0.33, 0.52, 0.18)");
    }

    #[test]
    fn a_return_closes_a_region_so_later_code_is_not_swept_up() {
        let src = SRC.replace("void main() {}", "void main() { Terrain z; z.bump = 9.0; }");
        let (assigns, _) = parse_tune(&src);
        assert!(!assigns.iter().any(|a| a.raw == "9.0"), "{:?}", assigns);
    }

    #[test]
    fn the_values_load_as_the_starting_point() {
        let shader = parse(SRC);
        assert!(shader.errors.is_empty(), "{:?}", shader.errors);
        let (assigns, _) = parse_tune(SRC);
        let mut values = UniformValues::default();
        let errs = seed_tune_values(&mut values, &assigns, &shader);
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(values.float.get("mats.terrain[0].bump"), Some(&0.35));
        assert_eq!(values.vec3.get("mats.terrain[1].colour").map(|v| v.x), Some(0.45));
        assert_eq!(values.float.get("globals.patch_frequency"), Some(&3.0));
    }

    #[test]
    fn a_path_no_block_member_matches_is_an_error() {
        let src = SRC.replace("k.bump   = 0.35;", "k.nonsense = 0.35;");
        let shader = parse(&src);
        let (assigns, _) = parse_tune(&src);
        let mut values = UniformValues::default();
        let errs = seed_tune_values(&mut values, &assigns, &shader);
        assert!(errs.iter().any(|e| e.contains("no block member has that path")), "{errs:?}");
    }

    #[test]
    fn save_rewrites_in_place_and_keeps_the_layout() {
        let shader = parse(SRC);
        let (assigns, _) = parse_tune(SRC);
        let mut values = UniformValues::default();
        values.float.insert("mats.terrain[0].bump".into(), 0.75);
        values.vec3.insert("mats.terrain[0].colour".into(), macroquad::prelude::vec3(0.1, 0.2, 0.3));

        let (out, saved) = write_tune_values(SRC, &assigns, &shader, &values).expect("a region");
        assert_eq!(saved, 2, "only the values that are set get written");
        assert!(out.contains("k.bump   = 0.75;    // keep this comment"), "{out}");
        assert!(out.contains("k.colour = vec3(0.1, 0.2, 0.3);"), "{out}");
        assert!(out.contains("k.bump   = 1.0;"), "the other element is untouched");
    }

    #[test]
    fn a_member_bound_to_the_bake_size_is_never_written_back() {
        let shader = parse(SRC);
        let (assigns, _) = parse_tune(SRC);
        let mut values = UniformValues::default();
        values.float.insert("globals.pixel_step".into(), 0.5);
        values.float.insert("globals.patch_frequency".into(), 9.0);

        let (out, saved) = write_tune_values(SRC, &assigns, &shader, &values).expect("a region");
        assert_eq!(saved, 1, "only the member the sliders actually own");
        assert!(out.contains("g.pixel_step = 0.001;"), "{out}");
        assert!(out.contains("g.patch_frequency = 9.0;"), "{out}");
    }

    #[test]
    fn a_crlf_file_stays_crlf() {
        let shader = parse(SRC);
        let mut values = UniformValues::default();
        values.float.insert("globals.patch_frequency".into(), 5.0);
        let crlf = SRC.replace('\n', "\r\n");
        let (crlf_assigns, _) = parse_tune(&crlf);
        let (out, _) = write_tune_values(&crlf, &crlf_assigns, &shader, &values).expect("a region");
        assert!(!out.replace("\r\n", "").contains('\n'), "mixed line endings");
        assert!(out.contains("g.patch_frequency = 5.0;"), "{out}");
    }

    /// The round trip on a real file, because the unit cases above all use a shape written to
    /// suit them. demo_pbr.glsl travels with the tool, so this stays true wherever it lands.
    #[test]
    fn the_shipped_example_round_trips_through_a_save() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/demo_pbr.glsl");
        let src = std::fs::read_to_string(path).expect("the example is in the crate");
        let shader = parse(&src);
        assert!(shader.errors.is_empty(), "{:?}", shader.errors);

        let (assigns, errs) = parse_tune(&src);
        assert!(errs.is_empty(), "{errs:?}");
        let mut values = UniformValues::default();
        assert!(seed_tune_values(&mut values, &assigns, &shader).is_empty());
        assert_eq!(values.float.get("u_mat.brick.mortar_width"), Some(&0.045));

        values.float.insert("u_mat.brick.mortar_width".into(), 0.09);
        let (out, saved) = write_tune_values(&src, &assigns, &shader, &values).expect("a region");
        assert_eq!(saved, assigns.len(), "every tuned member is written");

        // read the written file back: the new value loads, and nothing else moved
        let (assigns2, _) = parse_tune(&out);
        let mut reloaded = UniformValues::default();
        assert!(seed_tune_values(&mut reloaded, &assigns2, &parse(&out)).is_empty());
        assert_eq!(reloaded.float.get("u_mat.brick.mortar_width"), Some(&0.09));
        assert_eq!(reloaded.vec2.get("u_mat.brick.courses"), values.vec2.get("u_mat.brick.courses"));
        assert_eq!(out.lines().count(), src.lines().count(), "save must not shift any line");
    }

    #[test]
    fn a_file_with_no_region_says_so_rather_than_writing_nothing_quietly() {
        let shader = parse(SRC);
        let values = UniformValues::default();
        let err = write_tune_values(SRC, &[], &shader, &values).expect_err("no region");
        assert!(err.contains("no !(tune=...) region"), "{err}");
    }
}
