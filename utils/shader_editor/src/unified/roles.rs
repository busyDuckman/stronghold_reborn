// --------------------------------------------------------------------------------------------------------------------
// PBR roles: what each fragment output means, so the preview can light it instead of just
// showing you three buffers of noise.
//
// A modern material shader writes a G-buffer, and which lane of which target holds roughness
// is knowledge that otherwise lives only in the host. So the output says it, on its own line:
//
//   layout(location = 0) out vec4 o_albedo;    // !(pbr=albedo:rgb)
//   layout(location = 1) out vec4 o_normal;    // !(pbr=normal:xyz, pbr=height:w)
//   layout(location = 2) out vec4 o_surface;   // !(pbr=roughness:r, pbr=wetness:g, pbr=data:b)
//
// pbr= is the one repeatable key in the DSL: an output packs several channels, so it takes
// several terms. `normal` means the near-universal 0..1 packed encoding; `normal_signed` is
// there for a shader that writes -1..1 directly.

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    Albedo,
    Normal,
    NormalSigned,
    Height,
    Roughness,
    Metallic,
    Ao,
    Emissive,
    Opacity,
    Specular,
    Wetness,
    Data,
}

/// Every role, in the order the view buttons should offer them.
pub const ALL_ROLES: [Role; 12] = [
    Role::Albedo, Role::Normal, Role::NormalSigned, Role::Height, Role::Roughness,
    Role::Metallic, Role::Ao, Role::Emissive, Role::Opacity, Role::Specular,
    Role::Wetness, Role::Data,
];

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Role::Albedo => "albedo",
            Role::Normal => "normal",
            Role::NormalSigned => "normal_signed",
            Role::Height => "height",
            Role::Roughness => "roughness",
            Role::Metallic => "metallic",
            Role::Ao => "ao",
            Role::Emissive => "emissive",
            Role::Opacity => "opacity",
            Role::Specular => "specular",
            Role::Wetness => "wetness",
            Role::Data => "data",
        }
    }

    /// The spellings accepted in a pbr= term. basecolor and base_color are the UE5 habit.
    pub fn from_name(s: &str) -> Option<Role> {
        match s.trim().to_ascii_lowercase().as_str() {
            "albedo" | "basecolor" | "base_color" | "diffuse" => Some(Role::Albedo),
            "normal" => Some(Role::Normal),
            "normal_signed" | "normalsigned" => Some(Role::NormalSigned),
            "height" | "displacement" => Some(Role::Height),
            "roughness" => Some(Role::Roughness),
            "metallic" | "metalness" => Some(Role::Metallic),
            "ao" | "occlusion" | "ambient_occlusion" => Some(Role::Ao),
            "emissive" | "emission" => Some(Role::Emissive),
            "opacity" | "alpha" => Some(Role::Opacity),
            "specular" => Some(Role::Specular),
            "wetness" => Some(Role::Wetness),
            "data" | "mask" | "id" => Some(Role::Data),
            _ => None,
        }
    }

    /// How many components the role is made of. Data takes whatever it is given.
    pub fn components(self) -> Option<usize> {
        match self {
            Role::Albedo | Role::Normal | Role::NormalSigned | Role::Emissive => Some(3),
            Role::Height | Role::Roughness | Role::Metallic | Role::Ao
            | Role::Opacity | Role::Specular | Role::Wetness => Some(1),
            Role::Data => None,
        }
    }

    /// True for the roles the lit view actually shades with. Data and height do not light
    /// anything on their own, so they are view-only.
    pub fn lights(self) -> bool {
        !matches!(self, Role::Data | Role::Height)
    }

    /// What an absent role reads as in the lit view, so a shader that writes four of the
    /// twelve still previews sensibly instead of coming out black.
    pub fn fallback(self) -> [f32; 3] {
        match self {
            Role::Albedo => [0.8, 0.8, 0.8],
            Role::Normal | Role::NormalSigned => [0.0, 0.0, 1.0],
            Role::Roughness => [0.5, 0.0, 0.0],
            Role::Ao | Role::Opacity => [1.0, 0.0, 0.0],
            Role::Height | Role::Metallic | Role::Emissive
            | Role::Specular | Role::Wetness | Role::Data => [0.0, 0.0, 0.0],
        }
    }
}

/// One `pbr=<role>:<swizzle>` term: which output, and which of its components.
#[derive(Clone, Debug)]
pub struct RoleBind {
    pub role: Role,
    pub target: usize,          // the output's location, which is also its target index
    pub swizzle: Vec<usize>,    // component indices into that target, 0..3
    pub spelling: String,       // the swizzle as the author wrote it, so reports read it back
}

impl RoleBind {
    /// A binding with the swizzle spelled in xyzw, for anything built rather than parsed.
    pub fn new(role: Role, target: usize, swizzle: Vec<usize>) -> RoleBind {
        let spelling = swizzle_text(&swizzle);
        RoleBind { role, target, swizzle, spelling }
    }
}

/// Component indices back to their xyzw spelling.
pub fn swizzle_text(swizzle: &[usize]) -> String {
    swizzle.iter().map(|c| ["x", "y", "z", "w"][(*c).min(3)]).collect::<Vec<&str>>().concat()
}

/// Read one swizzle, in either the colour or the position spelling. Mixing them is a mistake
/// worth reporting rather than guessing at.
pub fn parse_swizzle(s: &str) -> Result<Vec<usize>, String> {
    let t = s.trim();
    if t.is_empty() { return Err("empty swizzle".to_string()); }
    if t.len() > 4 { return Err(format!("'{}' is longer than four components", t)); }
    let colour = "rgba";
    let position = "xyzw";
    let mut out = Vec::new();
    let mut set: Option<&str> = None;
    for c in t.chars() {
        let lower = c.to_ascii_lowercase();
        let which = if colour.contains(lower) { colour } else if position.contains(lower) { position } else {
            return Err(format!("'{}' is not a component of rgba or xyzw", c));
        };
        match set {
            Some(prev) if prev != which => return Err(format!("'{}' mixes the rgba and xyzw spellings", t)),
            _ => set = Some(which),
        }
        out.push(which.find(lower).expect("the set contains it"));
    }
    Ok(out)
}

/// Read a `pbr=` value: `<role>:<swizzle>`, or just `<role>` for a whole-output binding.
pub fn parse_role_term(value: &str, target: usize, target_components: usize, whose: &str)
    -> Result<RoleBind, String>
{
    let (role_txt, swz_txt) = match value.split_once(':') {
        Some((r, s)) => (r, Some(s)),
        None => (value, None),
    };
    let role = Role::from_name(role_txt)
        .ok_or_else(|| format!("{}: '{}' is not a PBR role", whose, role_txt.trim()))?;

    let swizzle = match swz_txt {
        Some(s) => parse_swizzle(s).map_err(|e| format!("{}: {}", whose, e))?,
        None => (0..role.components().unwrap_or(target_components).min(target_components)).collect(),
    };
    if let Some(want) = role.components() {
        if swizzle.len() != want {
            return Err(format!("{}: role {} needs {} component(s), got {}",
                whose, role.name(), want, swizzle.len()));
        }
    }
    if let Some(bad) = swizzle.iter().find(|c| **c >= target_components) {
        return Err(format!("{}: component {} is past the end of a {}-component output",
            whose, "xyzw".chars().nth(*bad).unwrap_or('?'), target_components));
    }
    // keep the author's spelling: a report that says rgb where the file says rgb is a report
    // you can match against the line it came from
    let spelling = match swz_txt {
        Some(t) => t.trim().to_ascii_lowercase(),
        None => swizzle_text(&swizzle),
    };
    Ok(RoleBind { role, target, swizzle, spelling })
}

/// Every role the shader declares, resolved to one binding each. A role claimed twice is an
/// error: the preview would have to pick one, and picking quietly is how you lose an hour.
pub fn resolve(binds: &[RoleBind]) -> (HashMap<Role, RoleBind>, Vec<String>) {
    let mut map: HashMap<Role, RoleBind> = HashMap::new();
    let mut errs = Vec::new();
    for b in binds {
        // Data is the one role several outputs may carry, since it means "no meaning".
        if b.role == Role::Data {
            map.entry(Role::Data).or_insert_with(|| b.clone());
            continue;
        }
        if let Some(prev) = map.get(&b.role) {
            errs.push(format!("pbr role {} is claimed by output {} and output {}",
                b.role.name(), prev.target, b.target));
            continue;
        }
        map.insert(b.role, b.clone());
    }
    if map.contains_key(&Role::Normal) && map.contains_key(&Role::NormalSigned) {
        errs.push("pbr roles normal and normal_signed are both declared; pick one encoding".to_string());
    }
    (map, errs)
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_spellings_land_on_the_same_role() {
        assert_eq!(Role::from_name("BaseColor"), Some(Role::Albedo));
        assert_eq!(Role::from_name("ambient_occlusion"), Some(Role::Ao));
        assert_eq!(Role::from_name("metalness"), Some(Role::Metallic));
        assert_eq!(Role::from_name("shinyness"), None);
    }

    #[test]
    fn swizzles_come_back_as_component_indices() {
        assert_eq!(parse_swizzle("rgb").expect("rgb"), vec![0, 1, 2]);
        assert_eq!(parse_swizzle("w").expect("w"), vec![3]);
        assert_eq!(parse_swizzle("zyx").expect("zyx"), vec![2, 1, 0]);
    }

    #[test]
    fn a_swizzle_prints_back_in_the_xyzw_spelling() {
        assert_eq!(swizzle_text(&[0, 1, 2]), "xyz");
        assert_eq!(swizzle_text(&[3]), "w");
        assert_eq!(RoleBind::new(Role::Albedo, 0, vec![0, 1, 2]).spelling, "xyz");
    }

    #[test]
    fn a_mixed_or_bogus_swizzle_is_reported() {
        assert!(parse_swizzle("rx").is_err());
        assert!(parse_swizzle("q").is_err());
        assert!(parse_swizzle("rgbaa").is_err());
        assert!(parse_swizzle("").is_err());
    }

    #[test]
    fn a_term_checks_its_component_count_against_the_role() {
        let b = parse_role_term("normal:xyz", 1, 4, "out o_normal").expect("normal");
        assert_eq!(b.role, Role::Normal);
        assert_eq!(b.swizzle, vec![0, 1, 2]);

        let e = parse_role_term("roughness:rg", 2, 4, "out o_s").expect_err("two lanes for one");
        assert!(e.contains("needs 1 component"), "{e}");
    }

    #[test]
    fn a_term_cannot_reach_past_the_output_it_sits_on() {
        let e = parse_role_term("height:w", 0, 3, "out o_n").expect_err("w of a vec3");
        assert!(e.contains("past the end"), "{e}");
    }

    #[test]
    fn a_bare_role_takes_the_leading_components() {
        let b = parse_role_term("albedo", 0, 4, "out o_a").expect("albedo");
        assert_eq!(b.swizzle, vec![0, 1, 2]);
        let b = parse_role_term("data", 3, 2, "out o_d").expect("data");
        assert_eq!(b.swizzle, vec![0, 1], "data takes whatever the output has");
    }

    #[test]
    fn a_role_claimed_twice_is_an_error_but_data_is_not() {
        let binds = vec![
            RoleBind::new(Role::Albedo, 0, vec![0, 1, 2]),
            RoleBind::new(Role::Albedo, 1, vec![0, 1, 2]),
            RoleBind::new(Role::Data, 2, vec![0]),
            RoleBind::new(Role::Data, 3, vec![1]),
        ];
        let (map, errs) = resolve(&binds);
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].contains("albedo"), "{errs:?}");
        assert!(map.contains_key(&Role::Data));
    }

    #[test]
    fn the_two_normal_encodings_cannot_both_be_declared() {
        let binds = vec![
            RoleBind::new(Role::Normal, 0, vec![0, 1, 2]),
            RoleBind::new(Role::NormalSigned, 1, vec![0, 1, 2]),
        ];
        let (_map, errs) = resolve(&binds);
        assert!(errs.iter().any(|e| e.contains("pick one encoding")), "{errs:?}");
    }
}
