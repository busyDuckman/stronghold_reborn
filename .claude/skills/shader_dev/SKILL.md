---
name: shader_dev
description: Use this skill whenever you create, edit, debug or visually iterate on a GLSL shader in this project - a paired .glsl/.vert, a unified GLSL 450 file with uniform blocks and PBR outputs, the shader_editor util, or the !(...) metadata hints. Triggers on "new shader", "tweak the shader", "run the shader editor", "render the shader", "why does this shader look wrong", "add a uniform", "bake a texture", "terrain.glsl". Read this BEFORE opening a shader file. Not for the WGSL strings inside landscape_shader/src.
---

# Developing GLSL shaders with shader_editor

## What this covers, and what it does not

`utils/shader_editor` is a GLSL tool. It reads two conventions and works out which from the
file itself, so there is no flag to pass and nothing to configure:

| | **paired** | **unified** |
|---|---|---|
| Files | `foo.glsl` + `foo.vert`, same stem, both required | one file, both stages behind `#ifdef` guards |
| GLSL | 330 | 450, as naga reads it |
| Uniforms | flat `uniform float u_x;` | `layout(std140, binding = 0) uniform B { ... } b;` |
| Outputs | one | as many as declared, each lane carrying a PBR role |
| Backend | macroquad | naga + wgpu, offscreen |
| Preview | the fragment colour | lit PBR, or any channel on its own, tiled |
| For | a 2D effect, a post pass, a transition | a material, a G-buffer, a texture bake |

Detection order: `// !(dialect=...)`, then a uniform block, then a stage guard, then
`#version 400`+. `--check` prints which it chose and why, on line one.

`landscape_shader/src/*.rs` holds WGSL strings; the editor cannot load those. But
`landscape_shader/landscape_textures/terrain.glsl` **is** a unified GLSL shader and the
editor drives it end to end. Do not confuse the two.

## Step 0 - read the grammar fresh

**Read `utils/shader_editor/README.md` before writing hints.** It is the live spec for the
`!(...)` metadata DSL, the std140 rules, the PBR roles, the tuning rules and the headless
flags. Do not work from a remembered version of it. If the tool and the README disagree, say
so - one of them is wrong.

## The hint DSL, in one breath

A uniform describes its own control, in its own comment. Nothing is configured anywhere else,
so the shader carries its editor UI with it and the two cannot drift apart.

```glsl
uniform vec2  u_resolution_px; // !(format=resolution) bound live from the preview size
uniform float u_time;          // !(format=TIME, max=8.0) sawtooth seconds, with a transport
uniform float u_percent;       // !(min=0, max=1, anim=4.0) sweeps min to max every 4s
uniform float u_thickness_px;  // !(default=3.0, min=1, max=6) notes after the paren
uniform sampler2D u_tex;       // !(default=checker, size=[256,256]) generated stand-in
```

- Give **every** uniform a hint with a sane `default=`, so a bare `--out render.png` already
  shows something. The hints are the contract any later rust wiring is written against.
- Hard-coding a constant is fine for scratch work. Promote it to a hinted uniform before
  calling the look done.
- Unknown keys and unparsable values are reported, not ignored: a typo shows up in `--check`.
- `step=` for anything the shader rounds anyway (a frequency, a count). `lanes=[a,b,c,d]` and
  `names=[..]` turn x/y/z/w and `[3]` into words, which is the difference between a hundred
  sliders and a usable panel.

## Writing a unified shader

The same hints, on block members instead of uniforms, plus four things that are only here.

```glsl
#ifdef VERTEX     // !(stage=vertex, verts=3)     omit the whole stage and you get a built-in one
#ifdef FRAGMENT   // !(stage=fragment)
#ifdef USE_DEFAULTS   // !(toggle="bake from the compiled-in table", default=off)

layout(std140, binding = 0) uniform Globals {
    float patch_frequency;   // !(min=1, max=16, step=1) one member per line, always
    float pixel_step;        // !(format=texel) bound from the bake size; never typed in
} globals;                   // the instance name is the path: globals.patch_frequency

layout(location = 1) out vec4 o_normal;   // !(pbr=normal:xyz, pbr=height:w)
```

Rules that bite:

- **One member per line.** Two declarations on one line is an error, not a skip: the editor
  packs the std140 buffer itself, and a dropped member shifts every offset after it.
- **`layout(std140, binding = N)` and an instance name are both required.**
- **Every output needs a `pbr=` role**, even if it is `pbr=data:rgba`. Without one the lit
  view has nothing to shade and the channel buttons have nothing to offer.
- `vec3` + `float` pack into one 16-byte slot. Use that; it is why the struct is the size you
  expect.
- Under wgpu, clip y is up while a texture's first row is its top. The convention here is
  **v = 0 on the first row**, so a hand-written vertex stage flips y:
  `gl_Position = vec4(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);`. Leave the flip
  out and the bake is upside down relative to everything that samples it. `gl_FragCoord`
  follows the target rather than GLSL's bottom-left origin, so work from the uv.

## Tuning: getting the numbers out of your head and into the file

Both conventions have a way to keep live sliders and still ship plain values.

- **paired**: `#ifdef EDITOR` with `t_` uniforms in the `#ifdef` arm and matching `const`s in
  the `#else` arm. The product compiles the consts.
- **unified**: a `// !(tune=<block.path>)` marker over a run of `k.field = value;` assignments,
  which is what a default table already looks like. The path starts at the block instance name.

In both, **Save tuning** writes the dialled-in values back in place, alignment and trailing
comments intact, and the saved value beats the `default=` hint on the next load. Tuned members
show green. This is the feature that turns "edit a number, rebuild, look" into a slider.

## Driving it

```
just shader-editor <file.glsl>                    interactive editor (for the user)
just shader-editor <file.glsl> --check            compile + metadata dump, the dialect on line one
just shader-editor <file.glsl> --out <file.png>   render one frame to a PNG
    --size <WxH>        render size (default: a resolution member's default=, else 512x512)
    --time <secs>       seat the clock: TIME uniforms wrap, anims sweep, integrals accumulate
    --set <name=value>  override a uniform, repeatable ("0.75", "1,2,3", "#rrggbb[aa]")
                        unified: the full path, --set globals.patch_frequency=6
    --channel <name>    unified: lit (default), a declared role, raw:N, or all for one file each
    --tiles <n>         unified: n by n, which is the only way to see whether a bake joins itself
    --toggle NAME=on    unified: set an #ifdef the shader offers
```

Exit code is non-zero on failure; compile errors with GLSL line numbers go to stdout.

- **A paired `--check` or `--out` flashes up a small GL window.** That is macroquad needing a
  real context, not a fault.
- **A unified `--check` needs no GPU** - naga does the compiling. A unified `--out` needs a
  Vulkan, Metal or DX12 adapter and says so if there is not one.

**The iteration loop:** edit -> `--check` until clean -> `--out` render -> Read the PNG to
inspect it -> repeat. Write PNGs to the scratchpad directory, not the repo; if one belongs in
the tree, it goes to `.output/`.

- Render **several states**, not one. A few `--time` values across the loop, the extremes and
  the middle of each ranged uniform, and a non-square `--size`. Animation bugs and edge
  artefacts hide in the states you did not render.
- For a unified shader, `--channel all` is the first thing to look at: an albedo that looks
  right over a normal map that is noise tells you which half is broken.
- For anything tiling, render `--tiles 2` before saying it works.
- To confirm something animates, render two `--time` values and compare them.
- The interactive GUI hot-reloads on save. A good pair session is the user keeping the window
  open while you edit the file, so they watch your changes land. Ask which mode they want if
  they are watching.

## Gotchas

- Never `fract(sin(...))` hashes. They glitch on some GPUs. Use a sinless integer hash.
- Time wraps. Motion has to survive the clock jumping back to 0.
- Speed that changes over time -> drive it with a `format=integral` uniform, never
  `speed * u_time`, or the whole history rescales every time speed changes.
- Uniform names are string-bound on the host side, and a unified block is bound by byte
  offset. A compiler cannot catch a shader/host mismatch, it just silently feeds garbage.
  After wiring or any rename, check `--check`'s offsets against the host's struct.
- A tiling shader needs whole-number frequencies and periodic noise. Round them in the shader
  and hint the control with `step=1`, so a slider cannot produce a seam.
- Unified targets are `Rgba8Unorm`. A channel needing more than 0..1 must scale itself.
- The paired preview renders offscreen at exact pixel size, so `gl_FragCoord` and any
  resolution uniform agree with how a product renders, rather than with the window.

## Where a shader lives

`utils/` is the tool, not a home for shaders. A shader belongs with the thing that draws it,
and a paired `.vert` sits beside its `.glsl` under the same stem. The editor takes an absolute
path, so a shader anywhere in the tree (or outside it) works.

Worked examples: `utils/shader_editor/examples/demo.glsl` (paired, every hint form),
`utils/shader_editor/examples/demo_pbr.glsl` (unified, small), and
`landscape_shader/landscape_textures/terrain.glsl` (unified, large: three uniform blocks, an
array of seven structs, three outputs, a toggle and tune regions).

Utils are support code: `unwrap()` and printing to stdout are fine inside `utils/`, and the
smell scanner knows it. That licence does not extend to the shader's host code.
