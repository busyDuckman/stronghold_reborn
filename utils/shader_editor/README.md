# shader_editor

Live GLSL editor. Point it at a shader and it gives you a preview that recompiles when you
save, a control for every uniform, and a way to write the values you dialled in back into the
file.

```bash
just shader-editor utils/shader_editor/examples/demo.glsl        # paired
just shader-editor utils/shader_editor/examples/demo_pbr.glsl    # unified
```

## Two conventions

A shader is written in one of two conventions, and **the file says which**. There is no flag
to remember and nothing to configure.

| | **paired** | **unified** |
|---|---|---|
| Files | `foo.glsl` + `foo.vert`, same stem | one file, both stages |
| GLSL | 330, as OpenGL reads it | 450, as naga reads it |
| Uniforms | flat `uniform float u_x;` | `layout(std140, binding = 0) uniform Block { ... } b;` |
| Outputs | one | as many as you declare, with PBR roles |
| Backend | macroquad | naga + wgpu, offscreen |
| Preview | the fragment colour | lit PBR, or any channel on its own |
| Tuning | `#ifdef EDITOR` / `#else` consts | `!(tune=)` assignment regions |
| For | a 2D effect, a post pass, a transition | a material, a G-buffer, a texture bake |

Which one you get is worked out from the file, in this order: an explicit
`// !(dialect=paired|unified)`, then a `layout(...) uniform` block, then an `#ifdef VERTEX` or
`#ifdef FRAGMENT` stage guard, then `#version 400` or above. Nothing matching means paired.
`--check` prints the answer and the reason, so a file read as the wrong one says so on line one.

The hint DSL below is shared. Everything about controls, defaults, ranges, motion and notes
works the same in both.

## The idea

A uniform describes its own control, in its own comment. Nothing is configured anywhere else,
so a shader carries its editor UI with it and cannot drift out of sync with one.

```glsl
uniform float u_thickness_px;  // !(default=3.0, min=1, max=3.1415) some notes
```

Everything inside `!(...)` is `key=value`. Anything after the closing paren is notes, shown
under the control. Unknown keys and unparsable values are reported in the Output panel rather
than ignored, so a typo in a hint is visible instead of silently doing nothing.

The same `!(...)` form annotates everything the editor reads: a block member, a fragment
output's PBR role, an `#ifdef` guard's stage or toggle. The keys differ by where it sits.

### Keys on a uniform or a block member

| Key | Means |
|-----|-------|
| `format=` | How to present it: `TIME`, `RGB`, `RGBA`, `RESOLUTION`, `TEXEL`, `INTEGRAL`, `MAT`. |
| `default=` | Starting value, checked against the declared type. |
| `min=` / `max=` | Slider range. Both present on a float also enables the Anim control. |
| `step=` | Snap the control to a multiple, for a value the shader needs whole. |
| `anim=<secs>` | Pre-enable Anim, sweeping min to max over this period. Needs a range. |
| `what=<uniform>` | For `format=integral`: the float being accumulated. |
| `size=[w,h]` | Pixel size of a generated `sampler2D` stand-in. (paired only) |
| `lanes=[a,b,..]` | Names for a vector's components, in place of x/y/z/w. |
| `names=[a,b,..]` | Names for an array's elements, one per element. |
| `group="name"` | Fold this member into a named, collapsing section. |

### Formats worth knowing

- **`format=resolution`** on a `vec2` is bound live from the preview or bake size, so resizing
  is reflected without touching anything.
- **`format=texel`** is the same idea for a step: on a float it is one texel in uv on the
  longer edge, on a `vec2` it is `1/w, 1/h`. That is the finite-difference step a normal map
  wants, and it is the one number that must follow the bake size rather than be typed in.
- **`format=TIME`** on a float is a sawtooth of seconds over `max=`, with Play / Pause /
  Restart / frame-jog. Scrubbing it moves the clock, so Play resumes from where you left it.
- **`format=integral, what=u_x`** accumulates `u_x * dt` each frame, the way a running
  integral behaves in a game loop. If the source is an Anim uniform, the integral resets when
  the sweep wraps, so a looping preview stays periodic instead of drifting upward forever.
- **`anim=`** is the one that saves the most time: instead of scrubbing a value by hand to see
  what the motion looks like, tick Anim and watch. Available on every ranged float, whether or
  not the shader asked for it.

A member the editor binds itself (`resolution`, `texel`) is never written back by Save: the
number in the file is the host's business and no slider ever held it.

## The paired convention

Two files with the same stem: `foo.glsl` (fragment) and `foo.vert` (vertex). Both are watched;
either one changing recompiles. `examples/demo.glsl` uses every form.

### Sampler stand-ins

A shader that samples a texture is useless to preview against nothing, and wiring the
project's asset pipeline into a dev tool makes the dev tool a liability. So the shader says
what it wants:

```glsl
uniform sampler2D u_banner;  // !(default=text:"Crossword", size=[512,128])
uniform sampler2D u_tex;     // !(default=art/standin.png)
uniform sampler2D u_fill;    // !(default=#334455ff, size=[64,64])
uniform sampler2D u_noise;   // !(default=checker)
```

`text:` renders in macroquad's built-in font, so the tool never touches the project's font
stack. A file path is tried relative to the cwd first, then relative to the `.glsl`. Anything
missing falls back to a checkerboard **and says so in the log**.

### Tuning blocks

Some shaders may not grow their uniform set: a background pass with a fixed material, for
instance, ships its tunables as `const`. That normally means editing numbers, rebuilding, and
looking, over and over. A tuning block gets you sliders anyway:

```glsl
#ifdef EDITOR   // !(group="craters")
uniform float t_crater_freq;   // !(default=2.0, min=0.5, max=8)
uniform vec2  t_crater_rad;    // !(default=[0.1,0.5])
#else
const float t_crater_freq = 0.5;
const vec2  t_crater_rad  = vec2(0.16, 0.50);
#endif
```

The editor compiles the `#ifdef` arm, so those are live uniforms it can drive. Your project
compiles the file exactly as written and gets the `#else` consts. **Save tuning** writes the
current slider values onto those consts.

- Tuning params show **green**, because those are the values Save commits. Everything else is
  live-only and is lost when you close the window.
- `!(group="name")` on the guard line collapses the block into one named section.
- A saved const beats the `default=` hint on load; the hint is never rewritten.
- Save rewrites only the text between `=` and `;`, so trailing comments and column alignment
  survive. A tuning uniform with no const yet gets one created above the `#endif`.
- Line endings are preserved, CRLF included.
- A const with no uniform, a type mismatch, or an unparsable value is an **error**: your
  project's compile is what is at stake, and it is not being tested here.

The guard line is swapped for `#if 1` rather than injecting a `#define`, so GLSL compile-error
line numbers still match the file on disk.

## The unified convention

One file, GLSL 450, compiled by naga and drawn by wgpu into one target per declared output.
This is the convention for anything you would write for wgpu, Vulkan or a modern engine: a
material, a G-buffer, a texture bake. `examples/demo_pbr.glsl` is a small complete one;
`landscape_shader/landscape_textures/terrain.glsl` is a large one.

### Stages

```glsl
#ifdef VERTEX     // !(stage=vertex, verts=3)
#ifdef FRAGMENT   // !(stage=fragment)
```

The file is compiled once per stage with only that stage's define set, which is exactly what
the host does. Guards literally named `VERTEX` and `FRAGMENT` are recognised without the
annotation; `!(stage=)` is for a file that calls them something else. `verts=` is how many
vertices the geometry pass draws, 3 by default, which is a fullscreen triangle.

**A shader with no vertex stage gets a built-in one**: a fullscreen triangle handing the
fragment stage a `v_uv` of 0..1. That is what `demo_pbr.glsl` relies on, and it is why a
procedural texture needs no boilerplate.

### Row order

Under wgpu, clip space has y up while a texture's first row is its top. The built-in vertex
stage flips y so that **v = 0 is the first row of every target**, which is how wgpu samples a
texture, how the editor reads the bake back, and how the PNG is written. What the preview
shows is what the PNG holds, and a bake sampled at the uv it was written with comes back the
same way up. Write your own vertex stage the same way unless the host wants the other
convention, and remember which you chose:

```glsl
gl_Position = vec4(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);   // v = 0 on row 0
gl_Position = vec4(corner * 2.0 - 1.0, 0.0, 1.0);                           // v = 0 on the last row
```

`gl_FragCoord` follows the target, not GLSL: naga does not emulate OpenGL's bottom-left
origin, so `gl_FragCoord.y` is 0.5 on the first row. Derive positions from the uv, not from
`gl_FragCoord`, and this never comes up.

### Uniform blocks

```glsl
layout(std140, binding = 0) uniform Globals {
    vec4 weight[7];         // !(lanes=[u0v0, u1v0, u0v1, u1v1], names=[grass, rocks, ...], min=0, max=1)
    float patch_frequency;  // !(min=1, max=16, step=1) patches across one tile edge
} globals;
```

The editor computes the std140 layout itself and packs the buffer, so it has to agree with the
GPU to the byte. That means some rules:

- **One member per line**, so each can carry its own `!(...)`. Two declarations on one line is
  an error, not a silent skip: a member the reader drops would shift every offset after it.
- **`layout(std140, binding = N)` and an instance name** are both required. The instance name
  is the path a member is addressed by: `globals.patch_frequency`, which is what the control
  shows on hover and what `--set` takes.
- A member type with no layout the editor knows is an error. `float`, `int`, `uint`, `vec2`,
  `vec3`, `vec4`, structs and arrays get controls. `mat2`/`mat3`/`mat4`/`bool` keep their slot
  and their offsets but get no control, and `--check` says so.
- An array size can be a literal or a `const int` declared in the same file.
- `vec3` takes 16 bytes of alignment but only 12 of size, so a `float` behind one packs into
  the gap. That is the trick every PBR struct uses and the editor implements it.

A `struct` member expands to one control per field. An **array of structs** becomes one
collapsing section per element, labelled from `names=`, which is what makes a seven-terrain-type
block navigable rather than a hundred anonymous sliders. There is a filter box above the list.

### Outputs and PBR roles

A G-buffer is only readable if something knows which lane means what, and that knowledge
otherwise lives only in the host. So the output says it:

```glsl
layout(location = 0) out vec4 o_albedo;    // !(pbr=albedo:rgb) rgb colour, a = 1
layout(location = 1) out vec4 o_normal;    // !(pbr=normal:xyz, pbr=height:w)
layout(location = 2) out vec4 o_surface;   // !(pbr=roughness:r, pbr=metallic:g, pbr=ao:b)
```

`pbr=` is the one repeatable key: an output packs several channels, so it takes several terms.
Each is `<role>:<swizzle>`, with the swizzle in either `rgba` or `xyzw` (mixing them is an
error). A bare `pbr=<role>` takes the leading components.

Roles: `albedo` (also `basecolor`, `base_color`, `diffuse`), `normal`, `normal_signed`,
`height` (also `displacement`), `roughness`, `metallic` (also `metalness`), `ao` (also
`occlusion`, `ambient_occlusion`), `emissive`, `opacity`, `specular`, `wetness`, and `data`
for a lane with no lighting meaning. `normal` is the packed 0..1 encoding every normal map
uses; `normal_signed` is for a shader that writes -1..1 directly.

Every output needs at least one role, even if it is `pbr=data:rgba`. A role claimed by two
outputs is an error, because the preview would otherwise have to pick one quietly.

### The preview

- **Lit** shades the G-buffer: one directional light, Lambert plus a GGX highlight, ambient
  scaled by occlusion, emissive added. Roles the shader does not declare fall back to sensible
  constants, so a shader writing four of the twelve channels still previews. It is there to
  tell you whether your normal map has the right sign and whether your roughness lane is in
  the range you think it is, not to match a renderer.
- **One button per declared role**, plus one per raw target. The buttons are generated from
  the roles, so they cannot claim a channel the shader does not write.
- **Tiles 1x1 / 2x2 / 4x4.** A tiling bake that does not join itself is something you will
  notice in the game and never at 1x1.
- **Bake px** is the render size, independent of the window. It feeds `format=resolution` and
  `format=texel`, so changing it is a real change to the bake, not a zoom.
- The render happens when something changes, not every frame. A unified shader is usually a
  bake, and terrain.glsl evaluates seven terrain types per pixel three times over.

### Toggles

```glsl
#ifdef USE_DEFAULTS   // !(toggle="bake from the compiled-in table", default=off)
```

A checkbox that recompiles, which is the same switch the host would set at build time. Put the
annotation on the first occurrence of the guard; the rest are left alone. Headless:
`--toggle USE_DEFAULTS=on`.

### Tuning: `!(tune=)` regions

A unified shader's blocks are filled by the host, so the values that ship with the shader live
in the code as plain assignments: a default table the host copies, or what the shader falls
back to under its defaults define. Those are the shader's real defaults, so they are what the
editor loads and what Save writes.

```glsl
Terrain default_terrain(int t) {
    Terrain k;
    switch (t) {
        case 0:   // !(tune=u_terrain.terrain[0]) grass
            k.cover_order = 2.0;
            k.bump        = 0.35;   // keep this comment
            break;
```

A region opens on the line carrying `!(tune=<path>)` and runs to the next such marker, or to
the first following `break;` or `return`, whichever comes first. Inside it, every
`<anything>.<field> = <value>;` line is the value of `<path>.<field>`. The path starts at the
block instance name, so it matches what `--check` prints.

- Tuned members show **green**, as tuning-block params do in the paired convention.
- A tune value beats the `default=` hint on load. The hint is never rewritten.
- Save rewrites only the text between `=` and `;`. Alignment, trailing comments and CRLF all
  survive.
- A path no block member matches is an **error**.
- A member bound from the bake size (`format=resolution`, `format=texel`) is skipped.
- The value has to be a literal or a `vecN(...)` constructor. `1.0 / 1024.0` is not read.

## From VS Code

The point of the tool is not to remember a path. Put the shader in one pane, run the task,
and it hot-reloads every time you save.

| Task | Does |
|------|------|
| **Shader Editor (active GLSL)** | Opens the editor window on it. Its own panel, not revealed. |
| **Shader Check (active GLSL)** | `--check` in the terminal: compile plus the metadata dump. No window. |
| **Shader Render (active GLSL)** | Every channel to `.output/`, for a look without a window. |

Reach them with `Ctrl+Shift+P` -> `Tasks: Run Task`. Worth binding the first one to a key,
since you will use it constantly. `Ctrl+Shift+P` -> `Preferences: Open Keyboard Shortcuts
(JSON)`, then:

```json
{ "key": "f7", "command": "workbench.action.tasks.runTask", "args": "Shader Editor (active GLSL)" }
```

There is also a **Debug Shader Editor (active GLSL)** launch config. That one is for
debugging the *tool* with a shader as its argument, not for debugging a shader. To just use
the tool, run the task.

If a shader lives outside this repo, the tasks still work: `${file}` is an absolute path and
the editor resolves the paired `.vert` next to it.

## Headless mode

For scripts, CI, and for an agent that cannot look at a window. Output goes to stdout and the
exit code is non-zero on failure.

```bash
shader_editor foo.glsl --check                      # compile + metadata + dump
shader_editor foo.glsl --out frame.png              # render one frame
shader_editor foo.glsl --out f.png --size 512x128   # explicit size
shader_editor foo.glsl --out f.png --time 2.5       # seat the clock at 2.5s
shader_editor foo.glsl --out f.png --set u_percent=0.9 --set u_bar_col=#00ff88
```

Unified shaders take three more:

```bash
shader_editor mat.glsl --out m.png --channel normal        # one channel
shader_editor mat.glsl --out m.png --channel raw:2         # one target, as written
shader_editor mat.glsl --out m.png --channel all           # m.lit.png, m.albedo.png, m.raw0.png, ...
shader_editor mat.glsl --out m.png --tiles 2               # 2x2, to see the seams
shader_editor mat.glsl --out m.png --toggle USE_DEFAULTS=on
shader_editor mat.glsl --out m.png --set globals.patch_frequency=6
```

`--time` evaluates the same motion the controls produce, at one instant: TIME uniforms wrap on
their loop, Anim uniforms sweep, integrals accumulate. `--set` takes `0.75`, `1,2,3`,
`#rrggbb`, `#rrggbbaa`, or a sampler source like `text:"Hi"`, and beats both the defaults and
`--time`. A `--set` naming a uniform that does not exist is an error in every mode.

That combination is what makes a contact sheet possible: loop over your shaders, render each
at a few `--time` values, and eyeball them side by side.

**A paired `--check` or `--out` flashes up a small GL window**, because macroquad needs a real
context. A **unified `--check` needs no GPU at all** (naga does the compiling), which is what
makes it usable on a build box; a unified `--out` does need a Vulkan, Metal or DX12 adapter
and says so plainly if there is not one.

## Notes

- The paired preview renders into an offscreen target at exact pixel size, so `gl_FragCoord`
  and any resolution uniform agree with how your project renders, rather than with the window.
- The paired on-screen blit flips vertically and its PNG path does not. `get_texture_data()`
  already flips rows into image convention during readback; `draw_texture_ex` samples the
  target raw. Verified against a shader with known orientation. Do not "fix" one to match the
  other.
- The unified path has no such split: the preview and the PNG are the same bytes in the same
  order. It writes its PNGs itself rather than through `Image::export_png`, which flips the
  rows on the way out. That flip is what the paired path wants, since its readback flips too
  and the two cancel; here the picture arrives in memory order and has to stay in it. Both
  orientations were measured against a shader that writes its own uv, not reasoned about.
- Slider values survive a recompile, so you can keep tuning while editing the source.
- The unified backend asks for a primary backend (Vulkan, Metal, DX12) and never presents: the
  GL backend would want the context macroquad is already holding. It renders offscreen and
  hands the pixels over, which costs one readback per render.
- Unified targets are `Rgba8Unorm`, because that is what a baked texture is. A channel needing
  more than 0..1 should scale itself and say so in its notes.
- This is a util, not product code: `unwrap` and printing to stdout are fine here, and the
  smell scanner knows it. See [../README.md](../README.md).
