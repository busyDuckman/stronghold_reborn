// --------------------------------------------------------------------------------------------------------------------
// std140 layout: where every member of a uniform block sits in its buffer.
//
// The editor has to agree with the GPU about this to the byte, so the rules are implemented
// rather than guessed at, and tested against the cases that actually bite:
//   - vec3 takes 16 bytes of alignment but only 12 of size, so a float behind one packs into
//     the gap. That is the trick every PBR struct uses, and getting it wrong silently shifts
//     every later member.
//   - an array element is always padded out to 16, however small the element is. A float[4]
//     is 64 bytes, not 16.
//   - a struct is padded out to its own alignment, which is at least 16.
//
// Expansion walks the declared members down to leaves (a scalar or a vector) and gives each
// one a byte offset and a key path like "terrain[3].cover_order". Those keys are what the
// controls, --set and the tune regions all address a value by.

use std::collections::HashMap;

use crate::glsl_meta::{ty_from_name, GLSLType, Hint};

/// A member's declared type: a leaf the editor can build a control for, a named struct to
/// walk into, or something whose layout is known but which gets no control (a matrix, a bool).
#[derive(Clone, Debug, PartialEq)]
pub enum MemberType {
    Leaf(GLSLType),
    Struct(String),
    Opaque { name: String, align: u32, size: u32 },
}

/// One declaration inside a block or a struct: `vec4 weight[7];  // !(...)`.
#[derive(Clone, Debug)]
pub struct FieldDecl {
    pub ty: MemberType,
    pub name: String,
    pub array: Option<u32>,
    pub hint: Hint,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<FieldDecl>,
}

pub type StructTable = HashMap<String, StructDef>;

/// One addressable value: a scalar or a vector at a known byte offset in its block's buffer.
#[derive(Clone, Debug)]
pub struct Leaf {
    pub key: String,               // the full path, eg "globals.terrain[3].cover_order"
    pub label: String,             // what the control row is called
    pub section: Option<String>,   // collapsing header this row belongs under, if any
    pub ty: GLSLType,
    pub offset: u32,               // byte offset within the block buffer
    pub hint: Hint,                // from the declaring field, so array elements share it
}

/// A member with a known layout but no control: recorded so --check can say so out loud.
#[derive(Clone, Debug)]
pub struct OpaqueLeaf {
    pub key: String,
    pub type_name: String,
    pub offset: u32,
    pub size: u32,
}

pub fn round_up(v: u32, to: u32) -> u32 {
    if to == 0 { return v; }
    ((v + to - 1) / to) * to
}

pub fn leaf_align(t: GLSLType) -> u32 {
    match t {
        GLSLType::Float | GLSLType::Int | GLSLType::Uint => 4,
        GLSLType::Vec2 => 8,
        GLSLType::Vec3 | GLSLType::Vec4 => 16,
        GLSLType::Sampler2D => 0,
    }
}

pub fn leaf_size(t: GLSLType) -> u32 {
    match t {
        GLSLType::Float | GLSLType::Int | GLSLType::Uint => 4,
        GLSLType::Vec2 => 8,
        GLSLType::Vec3 => 12,
        GLSLType::Vec4 => 16,
        GLSLType::Sampler2D => 0,
    }
}

/// The GLSL types that have a std140 layout but that the editor does not build a control for.
/// Matrices are column-major arrays of vec4 in std140, which is where the sizes come from.
pub fn opaque_type(name: &str) -> Option<MemberType> {
    let (align, size) = match name {
        "bool" => (4, 4),
        "mat2" => (16, 32),
        "mat3" => (16, 48),
        "mat4" => (16, 64),
        _ => return None,
    };
    Some(MemberType::Opaque { name: name.to_string(), align, size })
}

/// A block or struct member's type, from its GLSL spelling. `structs` is consulted last, so a
/// struct cannot shadow a built-in type name.
pub fn member_type(name: &str, structs: &StructTable) -> Option<MemberType> {
    if let Some(t) = ty_from_name(name) {
        if matches!(t, GLSLType::Sampler2D) { return None; }   // not a block member
        return Some(MemberType::Leaf(t));
    }
    if let Some(o) = opaque_type(name) { return Some(o); }
    if structs.contains_key(name) { return Some(MemberType::Struct(name.to_string())); }
    None
}

// --------------------------------------------------------------------------------------------------------------------
// The arithmetic.

pub fn type_align(ty: &MemberType, structs: &StructTable) -> Result<u32, String> {
    match ty {
        MemberType::Leaf(t) => Ok(leaf_align(*t)),
        MemberType::Opaque { align, .. } => Ok(*align),
        MemberType::Struct(name) => {
            let def = structs.get(name).ok_or_else(|| format!("unknown struct '{}'", name))?;
            let mut align = 16;    // rule 9: a struct is vec4-aligned at the least
            for f in &def.fields {
                align = align.max(field_align(f, structs)?);
            }
            Ok(align)
        }
    }
}

/// A field's alignment, which is its type's unless it is an array: an array element is always
/// padded out to a vec4 boundary.
fn field_align(f: &FieldDecl, structs: &StructTable) -> Result<u32, String> {
    let base = type_align(&f.ty, structs)?;
    Ok(if f.array.is_some() { round_up(base.max(16), 16) } else { base })
}

pub fn type_size(ty: &MemberType, structs: &StructTable) -> Result<u32, String> {
    match ty {
        MemberType::Leaf(t) => Ok(leaf_size(*t)),
        MemberType::Opaque { size, .. } => Ok(*size),
        MemberType::Struct(name) => {
            let def = structs.get(name).ok_or_else(|| format!("unknown struct '{}'", name))?;
            let mut off = 0u32;
            for f in &def.fields {
                let a = field_align(f, structs)?;
                off = round_up(off, a) + field_size(f, structs)?;
            }
            Ok(round_up(off, type_align(ty, structs)?))
        }
    }
}

/// One array element's stride: the element size padded out to the element's alignment.
pub fn array_stride(ty: &MemberType, structs: &StructTable) -> Result<u32, String> {
    let a = round_up(type_align(ty, structs)?.max(16), 16);
    Ok(round_up(type_size(ty, structs)?, a))
}

/// The bytes a whole field occupies, array included.
pub fn field_size(f: &FieldDecl, structs: &StructTable) -> Result<u32, String> {
    match f.array {
        Some(n) => Ok(array_stride(&f.ty, structs)? * n),
        None => type_size(&f.ty, structs),
    }
}

/// The size of a block holding these fields: every member laid out in turn, padded to 16.
pub fn block_size(fields: &[FieldDecl], structs: &StructTable) -> Result<u32, String> {
    let mut off = 0u32;
    for f in fields {
        off = round_up(off, field_align(f, structs)?) + field_size(f, structs)?;
    }
    Ok(round_up(off, 16))
}

// --------------------------------------------------------------------------------------------------------------------
// Expansion: every member down to the leaves that get a control.

/// What one element of an array is called, from the array's names= hint if it has one.
fn element_label(name: &str, index: usize, hint: &Hint) -> String {
    match hint.names.as_ref().and_then(|n| n.get(index)) {
        Some(tag) => format!("{}[{}] {}", name, index, tag),
        None => format!("{}[{}]", name, index),
    }
}

struct Walk<'a> {
    structs: &'a StructTable,
    leaves: Vec<Leaf>,
    opaques: Vec<OpaqueLeaf>,
    errors: Vec<String>,
}

impl Walk<'_> {
    /// One field at `base` bytes into the buffer, with `prefix` as its key path so far.
    /// `section` is the collapsing header its rows belong under, inherited by struct fields.
    fn field(&mut self, f: &FieldDecl, base: u32, prefix: &str, section: Option<String>) {
        let align = match field_align(f, self.structs) {
            Ok(a) => a,
            Err(e) => { self.errors.push(format!("{}{}: {}", prefix, f.name, e)); return; }
        };
        let at = round_up(base, align);
        match f.array {
            None => self.one(f, at, prefix, &f.name, section),
            Some(n) => {
                let stride = match array_stride(&f.ty, self.structs) {
                    Ok(s) => s,
                    Err(e) => { self.errors.push(format!("{}{}: {}", prefix, f.name, e)); return; }
                };
                for i in 0..n as usize {
                    let label = element_label(&f.name, i, &f.hint);
                    let elem_key = format!("{}[{}]", f.name, i);
                    // an array of structs becomes one collapsing section per element; an array
                    // of leaves stays a flat run of rows, since each element is a single control
                    let sect = match f.ty {
                        MemberType::Struct(_) => Some(format!("{}{}", prefix, label)),
                        _ => section.clone(),
                    };
                    self.one_named(f, at + stride * i as u32, prefix, &elem_key, &label, sect);
                }
            }
        }
    }

    fn one(&mut self, f: &FieldDecl, at: u32, prefix: &str, key: &str, section: Option<String>) {
        let label = f.name.clone();
        self.one_named(f, at, prefix, key, &label, section);
    }

    fn one_named(&mut self, f: &FieldDecl, at: u32, prefix: &str, key: &str, label: &str, section: Option<String>) {
        let full = format!("{}{}", prefix, key);
        match &f.ty {
            MemberType::Leaf(t) => self.leaves.push(Leaf {
                key: full,
                label: label.to_string(),
                section,
                ty: *t,
                offset: at,
                hint: f.hint.clone(),
            }),
            MemberType::Opaque { name, size, .. } => self.opaques.push(OpaqueLeaf {
                key: full,
                type_name: name.clone(),
                offset: at,
                size: *size,
            }),
            MemberType::Struct(sname) => {
                let Some(def) = self.structs.get(sname).cloned() else {
                    self.errors.push(format!("{}: unknown struct '{}'", full, sname));
                    return;
                };
                let inner_prefix = format!("{}.", full);
                let mut off = at;
                for sf in &def.fields {
                    let a = match field_align(sf, self.structs) {
                        Ok(a) => a,
                        Err(e) => { self.errors.push(format!("{}{}: {}", inner_prefix, sf.name, e)); return; }
                    };
                    let sect = sf.hint.group.clone().map(|g| format!("{}{}", inner_prefix, g)).or_else(|| section.clone());
                    self.field(sf, off, &inner_prefix, sect);
                    off = round_up(off, a) + match field_size(sf, self.structs) {
                        Ok(s) => s,
                        Err(e) => { self.errors.push(format!("{}{}: {}", inner_prefix, sf.name, e)); return; }
                    };
                }
            }
        }
    }
}

/// Every leaf of a block, with byte offsets, keyed under `prefix` (the block instance name
/// plus a dot). Also returns the members that have a layout but no control, and any problems.
pub fn expand(fields: &[FieldDecl], structs: &StructTable, prefix: &str)
    -> (Vec<Leaf>, Vec<OpaqueLeaf>, Vec<String>)
{
    let mut w = Walk { structs, leaves: Vec::new(), opaques: Vec::new(), errors: Vec::new() };
    let mut off = 0u32;
    for f in fields {
        let align = match field_align(f, structs) {
            Ok(a) => a,
            Err(e) => { w.errors.push(format!("{}{}: {}", prefix, f.name, e)); continue; }
        };
        let section = f.hint.group.clone().map(|g| format!("{}{}", prefix, g));
        w.field(f, off, prefix, section);
        off = round_up(off, align) + match field_size(f, structs) {
            Ok(s) => s,
            Err(e) => { w.errors.push(format!("{}{}: {}", prefix, f.name, e)); continue; }
        };
    }
    (w.leaves, w.opaques, w.errors)
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    fn field(ty: MemberType, name: &str, array: Option<u32>) -> FieldDecl {
        FieldDecl { ty, name: name.to_string(), array, hint: Hint::default(), line: 0 }
    }

    fn leaf(t: GLSLType, name: &str) -> FieldDecl { field(MemberType::Leaf(t), name, None) }

    #[test]
    fn scalars_and_vectors_have_the_spec_sizes() {
        assert_eq!((leaf_align(GLSLType::Float), leaf_size(GLSLType::Float)), (4, 4));
        assert_eq!((leaf_align(GLSLType::Vec2), leaf_size(GLSLType::Vec2)), (8, 8));
        assert_eq!((leaf_align(GLSLType::Vec3), leaf_size(GLSLType::Vec3)), (16, 12));
        assert_eq!((leaf_align(GLSLType::Vec4), leaf_size(GLSLType::Vec4)), (16, 16));
    }

    #[test]
    fn a_float_packs_into_the_tail_of_a_vec3() {
        let fields = vec![leaf(GLSLType::Vec3, "colour"), leaf(GLSLType::Float, "roughness")];
        let structs = StructTable::new();
        let (leaves, _, errs) = expand(&fields, &structs, "k.");
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(leaves[0].offset, 0);
        assert_eq!(leaves[1].offset, 12, "the float rides in the vec3's padding");
        assert_eq!(block_size(&fields, &structs).expect("size"), 16);
    }

    #[test]
    fn a_vec2_after_a_float_is_pushed_to_its_alignment() {
        let fields = vec![leaf(GLSLType::Float, "a"), leaf(GLSLType::Vec2, "b")];
        let structs = StructTable::new();
        let (leaves, _, _) = expand(&fields, &structs, "");
        assert_eq!(leaves[1].offset, 8);
        assert_eq!(block_size(&fields, &structs).expect("size"), 16);
    }

    #[test]
    fn every_array_element_is_padded_out_to_sixteen() {
        let structs = StructTable::new();
        let fields = vec![field(MemberType::Leaf(GLSLType::Float), "a", Some(4))];
        assert_eq!(block_size(&fields, &structs).expect("size"), 64, "float[4] is four vec4 slots");
        let (leaves, _, _) = expand(&fields, &structs, "");
        assert_eq!(leaves.iter().map(|l| l.offset).collect::<Vec<_>>(), vec![0, 16, 32, 48]);
    }

    #[test]
    fn the_terrain_shape_of_block_lays_out_as_the_gpu_sees_it() {
        // vec4 weight[7] then nine floats: the Globals block of terrain.glsl
        let structs = StructTable::new();
        let mut fields = vec![field(MemberType::Leaf(GLSLType::Vec4), "weight", Some(7))];
        for n in ["a", "b", "c", "d", "e", "f", "g", "h", "i"] {
            fields.push(leaf(GLSLType::Float, n));
        }
        let (leaves, _, errs) = expand(&fields, &structs, "globals.");
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(leaves[0].key, "globals.weight[0]");
        assert_eq!(leaves[6].offset, 96);
        assert_eq!(leaves[7].offset, 112, "the first float follows the array");
        assert_eq!(leaves[15].offset, 144);
        assert_eq!(block_size(&fields, &structs).expect("size"), 160);
    }

    #[test]
    fn an_array_of_structs_gets_one_section_per_element() {
        let mut structs = StructTable::new();
        structs.insert("Terrain".into(), StructDef {
            name: "Terrain".into(),
            fields: vec![leaf(GLSLType::Vec3, "colour"), leaf(GLSLType::Float, "rough")],
        });
        let mut decl = field(MemberType::Struct("Terrain".into()), "terrain", Some(3));
        decl.hint.names = Some(vec!["grass".into(), "rock".into(), "snow".into()]);

        let fields = vec![decl];
        assert_eq!(type_size(&MemberType::Struct("Terrain".into()), &structs).expect("size"), 16);
        assert_eq!(block_size(&fields, &structs).expect("size"), 48);

        let (leaves, _, errs) = expand(&fields, &structs, "t.");
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(leaves.len(), 6);
        assert_eq!(leaves[2].key, "t.terrain[1].colour");
        assert_eq!(leaves[2].offset, 16);
        assert_eq!(leaves[3].offset, 28, "the float still packs behind the vec3");
        assert_eq!(leaves[2].section.as_deref(), Some("t.terrain[1] rock"));
    }

    #[test]
    fn a_matrix_keeps_its_slot_without_getting_a_control() {
        let structs = StructTable::new();
        let fields = vec![
            field(opaque_type("mat4").expect("mat4"), "m", None),
            leaf(GLSLType::Float, "after"),
        ];
        let (leaves, opaques, errs) = expand(&fields, &structs, "");
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(opaques.len(), 1);
        assert_eq!(opaques[0].size, 64);
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaves[0].offset, 64, "the member after a matrix is not shifted");
    }

    #[test]
    fn an_unknown_struct_is_an_error_not_a_silent_skip() {
        let structs = StructTable::new();
        let fields = vec![field(MemberType::Struct("Missing".into()), "x", None)];
        let (_leaves, _op, errs) = expand(&fields, &structs, "b.");
        assert!(errs.iter().any(|e| e.contains("unknown struct")), "{errs:?}");
        assert!(block_size(&fields, &structs).is_err());
    }

    #[test]
    fn member_type_rejects_what_it_cannot_lay_out() {
        let structs = StructTable::new();
        assert_eq!(member_type("vec3", &structs), Some(MemberType::Leaf(GLSLType::Vec3)));
        assert!(matches!(member_type("mat3", &structs), Some(MemberType::Opaque { .. })));
        assert!(member_type("sampler2D", &structs).is_none());
        assert!(member_type("dvec4", &structs).is_none());
    }
}
