// --------------------------------------------------------------------------------------------------------------------
// Reading a unified shader: one file, both stages, uniform blocks, several fragment outputs.
//
// This is a line reader, not a GLSL compiler. It only needs the declarations, because naga
// does the compiling; what it has to get exactly right is the shape of the uniform blocks,
// since the editor packs their buffers itself. So one member per line is required, and
// anything it cannot lay out is an error rather than a skip: a member quietly dropped would
// shift every offset after it and the shader would read garbage with no complaint anywhere.
//
// What it picks up:
//   struct Foo { ... };                             a struct, for blocks that hold one
//   layout(std140, binding = 0) uniform G { ... } g; a uniform block and its members
//   layout(location = 1) out vec4 o_normal;          a fragment output and its pbr= roles
//   #ifdef VERTEX    // !(stage=vertex)              which define selects which stage
//   #ifdef SOMETHING // !(toggle="label")            a define the editor offers as a checkbox
//   const int TYPE_COUNT = 7;                        so an array can be sized by name

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

use crate::dialect::guards_define;
use crate::glsl_meta::{
    apply_control_token, check_default, hint_notes, hint_tokens, ty_from_name, GLSLType, Hint,
};
use crate::unified::roles::{parse_role_term, resolve, Role, RoleBind};
use crate::unified::std140::{
    block_size, expand, member_type, FieldDecl, Leaf, MemberType, OpaqueLeaf, StructDef, StructTable,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage { Vertex, Fragment }

#[derive(Clone, Debug)]
pub struct Block {
    pub name: String,          // the block's GLSL name, eg "Globals"
    pub instance: String,      // the instance name the shader reads through, eg "globals"
    pub binding: u32,
    pub group: u32,
    pub size: u32,             // std140 size, a multiple of 16
    pub leaves: Vec<Leaf>,
    pub opaques: Vec<OpaqueLeaf>,
    pub open_line: usize,
}

#[derive(Clone, Debug)]
pub struct Output {
    pub location: u32,
    pub name: String,
    pub ty: GLSLType,
    pub binds: Vec<RoleBind>,
    pub notes: String,
}

/// A define the shader offers as a checkbox. Flipping it recompiles, which is the point: it
/// is the same switch the host would set at build time.
#[derive(Clone, Debug)]
pub struct Toggle {
    pub define: String,
    pub label: String,
    pub on: bool,
}

#[derive(Clone, Debug)]
pub struct Unified {
    pub blocks: Vec<Block>,
    pub outputs: Vec<Output>,
    pub toggles: Vec<Toggle>,
    pub vertex_define: Option<String>,
    pub fragment_define: Option<String>,
    pub vertex_count: u32,     // how many vertices the geometry pass draws; a fullscreen triangle is 3
    pub roles: HashMap<Role, RoleBind>,
    pub errors: Vec<String>,
}

impl Default for Unified {
    fn default() -> Self {
        Self {
            blocks: Vec::new(), outputs: Vec::new(), toggles: Vec::new(),
            vertex_define: None, fragment_define: None, vertex_count: 3,
            roles: HashMap::new(), errors: Vec::new(),
        }
    }
}

impl Unified {
    /// Every controllable value in the shader, across all blocks, in declaration order.
    pub fn leaves(&self) -> impl Iterator<Item = (&Block, &Leaf)> {
        self.blocks.iter().flat_map(|b| b.leaves.iter().map(move |l| (b, l)))
    }

    pub fn find_leaf(&self, key: &str) -> Option<(&Block, &Leaf)> {
        self.leaves().find(|(_, l)| l.key == key)
    }

    pub fn toggle_on(&self, define: &str) -> bool {
        self.toggles.iter().any(|t| t.define == define && t.on)
    }
}

static STRUCT_OPEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*struct\s+([A-Za-z_]\w*)\s*\{").expect("literal regex"));
static BLOCK_OPEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:layout\s*\(([^)]*)\)\s*)?uniform\s+([A-Za-z_]\w*)\s*\{").expect("literal regex")
});
static DECL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*([A-Za-z_]\w*)\s+([A-Za-z_]\w*)\s*(?:\[\s*([A-Za-z_0-9]+)\s*\])?\s*;\s*$").expect("literal regex")
});
static OUT_DECL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:layout\s*\(([^)]*)\)\s*)?out\s+([A-Za-z_]\w*)\s+([A-Za-z_]\w*)\s*;\s*$").expect("literal regex")
});
static CONST_INT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*const\s+(?:int|uint)\s+([A-Za-z_]\w*)\s*=\s*(\d+)\s*[uU]?\s*;").expect("literal regex")
});

/// Split a source line into its code and its trailing comment.
pub fn split_comment(line: &str) -> (&str, &str) {
    match line.find("//") {
        Some(i) => (&line[..i], &line[i..]),
        None => (line, ""),
    }
}

/// True for the line that opens a uniform block. dialect detection leans on this, so it is
/// the one piece of block recognition that lives on its own.
pub fn is_block_open(line: &str) -> bool {
    let (code, _) = split_comment(line);
    BLOCK_OPEN.is_match(code)
}

// --------------------------------------------------------------------------------------------------------------------
// Preprocessor regions, so a vertex-stage declaration is not read as a fragment output.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Region { Vertex, Fragment, Neither }

struct Cond { stage: Option<Stage>, in_then: bool }

/// Which stage's arm each line sits in. A line in no stage guard at all is Neither, which is
/// where the shared declarations live.
fn regions(lines: &[&str], vertex: Option<&str>, fragment: Option<&str>) -> Vec<Region> {
    let mut out = Vec::with_capacity(lines.len());
    let mut stack: Vec<Cond> = Vec::new();
    for line in lines {
        let t = line.trim_start();
        if t.starts_with("#if") {
            let stage = match (vertex, fragment) {
                (Some(v), _) if guards_define(line, v) => Some(Stage::Vertex),
                (_, Some(f)) if guards_define(line, f) => Some(Stage::Fragment),
                _ => None,
            };
            stack.push(Cond { stage, in_then: true });
            out.push(current(&stack));
            continue;
        }
        if t.starts_with("#else") || t.starts_with("#elif") {
            if let Some(top) = stack.last_mut() { top.in_then = false; }
            out.push(current(&stack));
            continue;
        }
        if t.starts_with("#endif") {
            out.push(current(&stack));
            stack.pop();
            continue;
        }
        out.push(current(&stack));
    }
    out
}

fn current(stack: &[Cond]) -> Region {
    for c in stack.iter().rev() {
        let Some(s) = c.stage else { continue };
        if !c.in_then { return Region::Neither; }
        return match s { Stage::Vertex => Region::Vertex, Stage::Fragment => Region::Fragment };
    }
    Region::Neither
}

// --------------------------------------------------------------------------------------------------------------------
// The guards: which define selects a stage, and which ones the editor offers as checkboxes.

/// What the guards say: the two stage defines, how many vertices the vertex stage wants, and
/// the toggles.
/// A guard named VERTEX or FRAGMENT counts without annotation, because those are the names
/// every engine already uses; !(stage=) is for a file that calls them something else.
struct Guards {
    vertex: Option<String>,
    fragment: Option<String>,
    vertex_count: u32,
    toggles: Vec<Toggle>,
}

fn parse_guards(lines: &[&str]) -> (Guards, Vec<String>) {
    let mut vertex = None;
    let mut fragment = None;
    let mut vertex_count = 3u32;
    let mut toggles = Vec::new();
    let mut errs = Vec::new();

    for (i, line) in lines.iter().enumerate() {
        let (code, comment) = split_comment(line);
        let t = code.trim_start();
        let Some(rest) = t.strip_prefix("#ifdef") else { continue };
        if !rest.starts_with(char::is_whitespace) { continue; }
        let Some(define) = rest.trim_start().split_whitespace().next() else { continue };
        let whose = format!("#ifdef {} at line {}", define, i + 1);

        let mut stage: Option<Stage> = None;
        let mut toggle_label: Option<String> = None;
        let mut toggle_on = false;
        let mut saw_toggle = false;
        for tok in hint_tokens(comment).unwrap_or_default() {
            match tok.key.as_str() {
                "stage" => match tok.value.to_ascii_lowercase().as_str() {
                    "vertex" => stage = Some(Stage::Vertex),
                    "fragment" => stage = Some(Stage::Fragment),
                    other => errs.push(format!("{}: stage '{}' is not vertex or fragment", whose, other)),
                },
                "toggle" => {
                    saw_toggle = true;
                    toggle_label = Some(crate::glsl_meta::unquote(&tok.value).to_string());
                }
                "verts" => match tok.value.parse::<u32>() {
                    Ok(n) if n >= 1 => vertex_count = n,
                    _ => errs.push(format!("{}: verts '{}' is not a vertex count", whose, tok.value)),
                },
                "default" => match tok.value.to_ascii_lowercase().as_str() {
                    "on" | "1" | "true" => toggle_on = true,
                    "off" | "0" | "false" => toggle_on = false,
                    other => errs.push(format!("{}: default '{}' is not on or off", whose, other)),
                },
                _ => errs.push(format!("{}: unknown hint key '{}' (want stage=, verts= or toggle=)", whose, tok.key)),
            }
        }

        if stage.is_none() && !saw_toggle {
            match define {
                "VERTEX" => stage = Some(Stage::Vertex),
                "FRAGMENT" => stage = Some(Stage::Fragment),
                _ => continue,
            }
        }
        if saw_toggle && stage.is_some() {
            errs.push(format!("{}: a guard is either a stage or a toggle, not both", whose));
            continue;
        }
        match stage {
            Some(Stage::Vertex) => match &vertex {
                Some(prev) => errs.push(format!("{}: the vertex stage is already {}", whose, prev)),
                None => vertex = Some(define.to_string()),
            },
            Some(Stage::Fragment) => match &fragment {
                Some(prev) => errs.push(format!("{}: the fragment stage is already {}", whose, prev)),
                None => fragment = Some(define.to_string()),
            },
            None => {
                if toggles.iter().any(|t: &Toggle| t.define == define) { continue; }
                toggles.push(Toggle {
                    define: define.to_string(),
                    label: toggle_label.filter(|l| !l.is_empty()).unwrap_or_else(|| define.to_string()),
                    on: toggle_on,
                });
            }
        }
    }
    (Guards { vertex, fragment, vertex_count, toggles }, errs)
}

// --------------------------------------------------------------------------------------------------------------------
// Declarations.

/// One `key = value` out of a layout(...) qualifier list.
fn layout_value(qualifiers: &str, key: &str) -> Option<String> {
    for part in qualifiers.split(',') {
        let Some((k, v)) = part.split_once('=') else { continue };
        if k.trim().eq_ignore_ascii_case(key) { return Some(v.trim().to_string()); }
    }
    None
}

/// Read a member or field declaration line. Returns None for a line that is not one.
fn parse_decl(line: &str, line_no: usize, structs: &StructTable, consts: &HashMap<String, u32>, owner: &str,
    errs: &mut Vec<String>) -> Option<FieldDecl>
{
    let (code, comment) = split_comment(line);
    if code.trim().is_empty() { return None; }
    let Some(cap) = DECL.captures(code) else {
        // a declaration the reader cannot take apart must be shouted about, not skipped: the
        // offsets of everything after it depend on knowing its size
        let t = code.trim();
        if t.contains(';') && !t.starts_with('#') && !t.starts_with('}') {
            errs.push(format!("{} line {}: '{}' is not one member per line (a member carries its own !(...))",
                owner, line_no + 1, t));
        }
        return None;
    };
    let type_name = cap[1].to_string();
    let name = cap[2].to_string();
    let whose = format!("{}.{}", owner, name);

    let array = match cap.get(3).map(|m| m.as_str()) {
        None => None,
        Some(tok) => match tok.parse::<u32>() {
            Ok(n) => Some(n),
            Err(_) => match consts.get(tok) {
                Some(n) => Some(*n),
                None => {
                    errs.push(format!("{}: array size '{}' is not a number or a const int in this file", whose, tok));
                    return None;
                }
            },
        },
    };
    if array == Some(0) {
        errs.push(format!("{}: array size 0", whose));
        return None;
    }

    let Some(ty) = member_type(&type_name, structs) else {
        errs.push(format!("{}: type '{}' has no std140 layout the editor knows", whose, type_name));
        return None;
    };

    let mut hint = Hint::default();
    for tok in hint_tokens(comment).unwrap_or_default() {
        if tok.bare {
            errs.push(format!("{}: unknown flag '{}' (want key=value)", whose, tok.key));
            continue;
        }
        if !apply_control_token(&mut hint, &tok, &whose, errs) {
            errs.push(format!("{}: unknown hint key '{}'", whose, tok.key));
        }
    }
    hint.notes = hint_notes(comment);
    if let MemberType::Leaf(t) = ty {
        check_default(t, &hint, &whose, errs);
    }
    if let (Some(n), Some(names)) = (array, hint.names.as_ref()) {
        if names.len() != n as usize {
            errs.push(format!("{}: names= has {} entries for an array of {}", whose, names.len(), n));
        }
    }
    if hint.lanes.is_some() {
        let want = match ty { MemberType::Leaf(t) => crate::glsl_meta::ty_components(t), _ => 0 };
        let got = hint.lanes.as_ref().map(|l| l.len()).unwrap_or(0);
        if got != want {
            errs.push(format!("{}: lanes= has {} names for a {}-component member", whose, got, want));
        }
    }

    Some(FieldDecl { ty, name, array, hint, line: line_no })
}

/// The `const int NAME = n;` values, so an array can be sized by name.
fn const_ints(lines: &[&str]) -> HashMap<String, u32> {
    let mut out = HashMap::new();
    for line in lines {
        let (code, _) = split_comment(line);
        if let Some(cap) = CONST_INT.captures(code) {
            if let Ok(n) = cap[2].parse::<u32>() { out.insert(cap[1].to_string(), n); }
        }
    }
    out
}

/// Every `struct Name { ... };` in the file. Structs are read before blocks, so a block can
/// hold one; a struct holding a struct works as long as the inner one is declared first.
fn parse_structs(lines: &[&str], consts: &HashMap<String, u32>, errs: &mut Vec<String>) -> StructTable {
    let mut table = StructTable::new();
    let mut i = 0usize;
    while i < lines.len() {
        let (code, _) = split_comment(lines[i]);
        let Some(cap) = STRUCT_OPEN.captures(code) else { i += 1; continue };
        let name = cap[1].to_string();
        let mut fields = Vec::new();
        let mut j = i + 1;
        let mut closed = false;
        while j < lines.len() {
            let (body, _) = split_comment(lines[j]);
            if body.trim_start().starts_with('}') { closed = true; break; }
            if let Some(f) = parse_decl(lines[j], j, &table, consts, &format!("struct {}", name), errs) {
                fields.push(f);
            }
            j += 1;
        }
        if !closed {
            errs.push(format!("struct {} at line {}: no closing brace", name, i + 1));
            break;
        }
        if table.contains_key(&name) {
            errs.push(format!("struct {}: declared twice", name));
        }
        table.insert(name.clone(), StructDef { name, fields });
        i = j + 1;
    }
    table
}

/// Every uniform block, laid out and expanded to leaves.
fn parse_blocks(lines: &[&str], structs: &StructTable, consts: &HashMap<String, u32>, errs: &mut Vec<String>)
    -> Vec<Block>
{
    let mut blocks = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        let (code, _) = split_comment(lines[i]);
        let Some(cap) = BLOCK_OPEN.captures(code) else { i += 1; continue };
        let qualifiers = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        let name = cap[2].to_string();
        let whose = format!("uniform block {}", name);

        let binding = match layout_value(qualifiers, "binding").map(|v| v.parse::<u32>()) {
            Some(Ok(b)) => Some(b),
            Some(Err(_)) => { errs.push(format!("{}: binding is not a number", whose)); None }
            None => { errs.push(format!("{}: needs an explicit layout(binding = N)", whose)); None }
        };
        let group = layout_value(qualifiers, "set").or_else(|| layout_value(qualifiers, "group"))
            .and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
        if !qualifiers.split(',').any(|q| q.trim().eq_ignore_ascii_case("std140")) {
            errs.push(format!("{}: mark it layout(std140, ...); the editor packs std140 and will not guess", whose));
        }

        let mut fields = Vec::new();
        let mut j = i + 1;
        let mut instance = String::new();
        let mut closed = false;
        while j < lines.len() {
            let (body, _) = split_comment(lines[j]);
            let t = body.trim_start();
            if t.starts_with('}') {
                closed = true;
                instance = t.trim_start_matches('}').trim().trim_end_matches(';').trim().to_string();
                break;
            }
            if let Some(f) = parse_decl(lines[j], j, structs, consts, &whose, errs) { fields.push(f); }
            j += 1;
        }
        if !closed {
            errs.push(format!("{} at line {}: no closing brace", whose, i + 1));
            break;
        }
        if instance.is_empty() {
            // GLSL lets a block have no instance name, but then its members are addressed
            // bare and the editor's keys would collide across blocks. Ask for one.
            errs.push(format!("{}: give the block an instance name (}} globals;), so its members have a path", whose));
            instance = name.to_lowercase();
        }

        let prefix = format!("{}.", instance);
        let (leaves, opaques, mut expand_errs) = expand(&fields, structs, &prefix);
        errs.append(&mut expand_errs);
        let size = match block_size(&fields, structs) {
            Ok(s) => s,
            Err(e) => { errs.push(format!("{}: {}", whose, e)); 16 }
        };

        if let Some(binding) = binding {
            blocks.push(Block { name, instance, binding, group, size, leaves, opaques, open_line: i });
        }
        i = j + 1;
    }

    for a in 0..blocks.len() {
        for b in (a + 1)..blocks.len() {
            if blocks[a].binding == blocks[b].binding && blocks[a].group == blocks[b].group {
                errs.push(format!("uniform blocks {} and {} share binding {}",
                    blocks[a].name, blocks[b].name, blocks[a].binding));
            }
            if blocks[a].instance == blocks[b].instance {
                errs.push(format!("uniform blocks {} and {} share the instance name {}",
                    blocks[a].name, blocks[b].name, blocks[a].instance));
            }
        }
    }
    blocks
}

/// The fragment outputs, with the PBR roles they carry.
fn parse_outputs(lines: &[&str], regions: &[Region], errs: &mut Vec<String>) -> Vec<Output> {
    let mut outs: Vec<Output> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if regions.get(i) == Some(&Region::Vertex) { continue; }   // that one is a varying, not a target
        let (code, comment) = split_comment(line);
        let Some(cap) = OUT_DECL.captures(code) else { continue };
        let type_name = cap[2].to_string();
        let name = cap[3].to_string();
        let whose = format!("out {}", name);
        let Some(ty) = ty_from_name(&type_name) else {
            errs.push(format!("{}: output type '{}' is not one the editor can read back", whose, type_name));
            continue;
        };
        let location = match cap.get(1).and_then(|q| layout_value(q.as_str(), "location")) {
            Some(v) => match v.parse::<u32>() {
                Ok(n) => n,
                Err(_) => { errs.push(format!("{}: location is not a number", whose)); continue; }
            },
            None => 0,
        };

        let components = crate::glsl_meta::ty_components(ty);
        let mut binds = Vec::new();
        for tok in hint_tokens(comment).unwrap_or_default() {
            if tok.key != "pbr" {
                errs.push(format!("{}: unknown hint key '{}' (an output takes pbr= only)", whose, tok.key));
                continue;
            }
            match parse_role_term(&tok.value, location as usize, components, &whose) {
                Ok(b) => binds.push(b),
                Err(e) => errs.push(e),
            }
        }
        if binds.is_empty() {
            errs.push(format!("{}: no pbr= role, so the preview cannot light it (use pbr=data:{} to say it has no meaning)",
                whose, "rgba".get(..components).unwrap_or("r")));
        }
        if let Some(prev) = outs.iter().find(|o| o.location == location) {
            errs.push(format!("{}: location {} is already {}", whose, location, prev.name));
            continue;
        }
        outs.push(Output { location, name, ty, binds, notes: hint_notes(comment) });
    }
    outs.sort_by_key(|o| o.location);
    outs
}

// --------------------------------------------------------------------------------------------------------------------
// The whole file.

/// Read a unified shader. Every problem found is collected rather than thrown, so --check can
/// print the lot in one go instead of one per run.
pub fn parse(src: &str) -> Unified {
    let lines: Vec<&str> = src.lines().collect();
    let mut errors = Vec::new();

    let (guards, mut guard_errs) = parse_guards(&lines);
    errors.append(&mut guard_errs);
    let Guards { vertex: vertex_define, fragment: fragment_define, vertex_count, toggles } = guards;

    let regs = regions(&lines, vertex_define.as_deref(), fragment_define.as_deref());
    let consts = const_ints(&lines);
    let structs = parse_structs(&lines, &consts, &mut errors);
    let blocks = parse_blocks(&lines, &structs, &consts, &mut errors);
    let outputs = parse_outputs(&lines, &regs, &mut errors);

    if outputs.is_empty() {
        errors.push("no fragment output found: a unified shader writes at least one out vec4".to_string());
    }
    let binds: Vec<RoleBind> = outputs.iter().flat_map(|o| o.binds.iter().cloned()).collect();
    let (roles, mut role_errs) = resolve(&binds);
    errors.append(&mut role_errs);

    Unified { blocks, outputs, toggles, vertex_define, fragment_define, vertex_count, roles, errors }
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "\
#version 450
const int TYPE_COUNT = 3;

struct Terrain {
    vec3 colour;        // !(format=RGB, default=#808080)
    float roughness;    // !(default=0.5, min=0, max=1)
};

layout(std140, binding = 0) uniform Globals {
    vec4 weight[TYPE_COUNT];    // !(lanes=[u0v0, u1v0, u0v1, u1v1], names=[grass, rock, snow], min=0, max=1)
    float patch_frequency;      // !(default=3, min=1, max=12, step=1) patches across the tile
} globals;

layout(std140, binding = 1) uniform TerrainBlock {
    Terrain terrain[TYPE_COUNT];   // !(names=[grass, rock, snow])
} mats;

#ifdef VERTEX
layout(location = 0) out vec2 v_uv;
void main() {}
#endif

#ifdef FRAGMENT
layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 o_albedo;    // !(pbr=albedo:rgb) the colour
layout(location = 1) out vec4 o_normal;    // !(pbr=normal:xyz, pbr=height:w)
#ifdef USE_DEFAULTS   // !(toggle=\"compiled-in defaults\", default=off)
#endif
void main() {}
#endif
";

    #[test]
    fn stages_and_toggles_come_off_the_guards() {
        let u = parse(SRC);
        assert!(u.errors.is_empty(), "{:?}", u.errors);
        assert_eq!(u.vertex_define.as_deref(), Some("VERTEX"));
        assert_eq!(u.fragment_define.as_deref(), Some("FRAGMENT"));
        assert_eq!(u.toggles.len(), 1);
        assert_eq!(u.toggles[0].define, "USE_DEFAULTS");
        assert_eq!(u.toggles[0].label, "compiled-in defaults");
        assert!(!u.toggles[0].on);
    }

    #[test]
    fn a_vertex_stage_varying_is_not_read_as_a_target() {
        let u = parse(SRC);
        assert_eq!(u.outputs.len(), 2, "{:?}", u.outputs.iter().map(|o| &o.name).collect::<Vec<_>>());
        assert_eq!(u.outputs[0].name, "o_albedo");
        assert_eq!(u.outputs[1].name, "o_normal");
    }

    #[test]
    fn blocks_are_laid_out_and_keyed_by_instance() {
        let u = parse(SRC);
        let g = u.blocks.iter().find(|b| b.name == "Globals").expect("Globals");
        assert_eq!((g.binding, g.size), (0, 64));
        assert_eq!(g.leaves.len(), 4);
        assert_eq!(g.leaves[0].key, "globals.weight[0]");
        assert_eq!(g.leaves[0].label, "weight[0] grass");
        assert_eq!(g.leaves[3].key, "globals.patch_frequency");
        assert_eq!(g.leaves[3].offset, 48);
        assert_eq!(g.leaves[3].hint.step, Some(1.0));

        let m = u.blocks.iter().find(|b| b.name == "TerrainBlock").expect("TerrainBlock");
        assert_eq!(m.leaves.len(), 6);
        assert_eq!(m.leaves[2].key, "mats.terrain[1].colour");
        assert_eq!(m.leaves[2].section.as_deref(), Some("mats.terrain[1] rock"));
        assert_eq!(m.size, 48);
    }

    #[test]
    fn roles_resolve_across_the_outputs() {
        let u = parse(SRC);
        assert_eq!(u.roles.len(), 3);
        let n = u.roles.get(&Role::Normal).expect("normal");
        assert_eq!((n.target, n.swizzle.clone()), (1, vec![0, 1, 2]));
        assert_eq!(u.roles.get(&Role::Height).expect("height").swizzle, vec![3]);
    }

    #[test]
    fn two_members_on_one_line_is_an_error() {
        let src = SRC.replace("float patch_frequency;      ", "float a; float b;  ");
        let u = parse(&src);
        assert!(u.errors.iter().any(|e| e.contains("one member per line")), "{:?}", u.errors);
    }

    #[test]
    fn a_member_type_with_no_layout_is_an_error() {
        let src = SRC.replace("float patch_frequency;", "dvec3 nope;");
        let u = parse(&src);
        assert!(u.errors.iter().any(|e| e.contains("no std140 layout")), "{:?}", u.errors);
    }

    #[test]
    fn a_block_needs_a_binding_std140_and_an_instance_name() {
        let src = SRC.replace("layout(std140, binding = 0) uniform Globals {", "uniform Globals {");
        let u = parse(&src);
        assert!(u.errors.iter().any(|e| e.contains("explicit layout(binding")), "{:?}", u.errors);
        assert!(u.errors.iter().any(|e| e.contains("std140")), "{:?}", u.errors);

        let src = SRC.replace("} globals;", "};");
        let u = parse(&src);
        assert!(u.errors.iter().any(|e| e.contains("instance name")), "{:?}", u.errors);
    }

    #[test]
    fn an_output_with_no_role_is_reported() {
        let src = SRC.replace("// !(pbr=albedo:rgb) the colour", "// just a colour");
        let u = parse(&src);
        assert!(u.errors.iter().any(|e| e.contains("no pbr= role")), "{:?}", u.errors);
    }

    #[test]
    fn names_must_match_the_array_length() {
        let src = SRC.replace("names=[grass, rock, snow])\n} mats;", "names=[grass, rock])\n} mats;");
        let u = parse(&src);
        assert!(u.errors.iter().any(|e| e.contains("names= has 2 entries for an array of 3")), "{:?}", u.errors);
    }

    #[test]
    fn a_guard_cannot_be_a_stage_and_a_toggle_at_once() {
        let src = SRC.replace("#ifdef VERTEX\n", "#ifdef VERTEX   // !(stage=vertex, toggle=\"no\")\n");
        let u = parse(&src);
        assert!(u.errors.iter().any(|e| e.contains("either a stage or a toggle")), "{:?}", u.errors);
    }

    #[test]
    fn duplicate_locations_and_bindings_are_caught() {
        let src = SRC.replace("layout(location = 1) out vec4 o_normal;", "layout(location = 0) out vec4 o_normal;");
        let u = parse(&src);
        assert!(u.errors.iter().any(|e| e.contains("is already o_albedo")), "{:?}", u.errors);

        let src = SRC.replace("binding = 1) uniform TerrainBlock", "binding = 0) uniform TerrainBlock");
        let u = parse(&src);
        assert!(u.errors.iter().any(|e| e.contains("share binding 0")), "{:?}", u.errors);
    }

    #[test]
    fn an_array_sized_by_an_unknown_name_is_an_error() {
        let src = SRC.replace("vec4 weight[TYPE_COUNT];", "vec4 weight[NOPE];");
        let u = parse(&src);
        assert!(u.errors.iter().any(|e| e.contains("is not a number or a const int")), "{:?}", u.errors);
    }

    #[test]
    fn is_block_open_ignores_a_commented_out_block() {
        assert!(is_block_open("layout(std140, binding = 0) uniform G {"));
        assert!(!is_block_open("// layout(std140, binding = 0) uniform G {"));
        assert!(!is_block_open("uniform float u_a;"));
    }
}
