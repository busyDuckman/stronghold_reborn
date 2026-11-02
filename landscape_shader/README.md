# landscape_shader

A self-contained test bed for a terrain renderer. It may become the game's renderer **or be thrown away**, so it is its own crate outside the root workspace and off the root `just vet`
path. Run it from here (`just run`) or from the repo root (`just landscape`).

## What it does

- Holds a world with no simulation: a game map (cells, corner heights, water, ground types,
  trees, buildings) and a list of units, plus a finer DEM derived from the map for rendering.
- File -> New -> Stronghold world rolls one of the original game's five named worlds with a
  faithful port of its generator (`src/stronghold_gen.rs`). Heights, water and rivers match a
  world captured from the real executable byte for byte; tree and building placement is close
  but not exact because three routines the generator asks are not recovered yet (see the
  "Stand-ins" comment in that file).
- File -> New -> Custom makes a map of any size from seeded noise.
- The lower viewport is a top-down view drawn by its own wgpu pass; the View tab toggles its
  layers. The upper viewport is drawn by whichever renderer the Renderer tab's combo picks.
- A Doom-style camera (position, heading, height, FOV, no pitch) walks with the arrow keys:
  Up/Down walk, Left/Right turn, Q/E strafe, PgUp/PgDn height, +/- FOV, Shift faster. The
  Camera tab sets whether it follows the ground or holds an altitude.
- Renderers implement the ViewportRenderer trait (`src/renderer.rs`). Two ship: Empty, and
  BasicRenderer3d, a terrain mesh from either the DEM or the game map cells with the game's
  own sprites standing on it as billboards, cut from STRONG.DAT under a chosen palette.
- A skybox (the Skybox trait, `src/skybox.rs`) hands the renderer an image for the camera to
  put behind the terrain. The game's nine painted skies (`src/backdrop.rs`) are one, as a
  still or as a panorama that pans with the heading (how many degrees the loop spans is a
  knob); None is the other. The Renderer tab
  picks the skybox and its knobs.

## Layout

```
landscape_shader/
|-- src/
|   |-- camera.rs        # the Doom-style camera, its keys and its matrices
|   |-- renderer.rs      # the ViewportRenderer trait and the Empty renderer
|   |-- basic_3d.rs      # BasicRenderer3d: terrain mesh + sprite billboards
|   |-- skybox.rs        # the Skybox trait: an image for the camera, blitted behind the terrain
|   |-- backdrop.rs      # the game's painted skies as a skybox
|   |-- dat.rs           # STRONG.DAT: index, palettes, the image decoder
|   |-- sprite_atlas.rs  # the entries the billboards use, packed under one palette
|   |-- game_map.rs      # the map: cells, heights, terrain bits, ground types, decor
|   |-- sprite.rs        # a unit, shaped like the original's records
|   |-- world.rs         # map + units + DEM
|   |-- dem.rs           # the high resolution height field derived from the map
|   |-- rng.rs           # the original game's random generator, exact
|   |-- stronghold_gen.rs# the original map roll, ported from reversing/src/world_gen.c
|   |-- custom_gen.rs    # the free-form generator
|   |-- noise.rs         # value noise and fractal noise
|   |-- top_down.rs      # the top-down view as geometry, no GPU
|   |-- gpu.rs           # device, offscreen targets, the top-down pipeline
|   |-- ui.rs            # egui: menu, sidebar, viewports, dialogs
|   |-- app.rs           # winit + egui + wgpu per-frame loop
|   \-- main.rs          # a shell around the lib
|-- examples/peek_seeds.rs   # the five named worlds as ASCII
|-- examples/peek_dat.rs     # how a few STRONG.DAT sheets were cut
|-- examples/peek_backdrops.rs # the evidence for how the painted skies are stored
|-- test_data/           # the seed-100 world captured from the real game, for the port test
|-- landscape_textures/  # terrain.glsl: bakes tileable ground textures from seven terrain types
\-- justfile
```

## Sprites

The container is read from `achieve/archive.org/en_cdrom/unpacked/STRONG.DAT` (gitignored;
copy it from zeta). Without it the app still runs and the billboards are flat colours. Which
entry stands for which tree, building or unit kind is a stand-in table in `sprite_atlas.rs`;
which palette is right for a sprite is an open question of the reversing pass, so the
Renderer tab lets you pick one.

The painted skies are entries 944 to 970: nine sets of two 320x60 images with a 48-colour
palette piece between them, stamped over slots 160..=207 of the main palette (741) before
the images are drawn. The two images of a set join into one 640-pixel loop. Both facts were
checked against the container (`cargo run --example peek_backdrops`) rather than assumed;
which set the game showed when is not recovered, so the Renderer tab lets you pick.

## Ground textures

`landscape_textures/terrain.glsl` is an offline baking shader, not part of the live renderer
yet. It takes seven terrain amounts per tile corner (grass, rocks, gravel, snow, sand, dirt,
underwater sand) and lays them down as patches in a cover order, writing albedo, normal and
a surface map. Its header documents the uniform blocks, the defaults and the tiling rule.
Nothing in this crate drives it yet; it compiles under wgpu's naga GLSL front end with
`VERTEX` or `FRAGMENT` defined.

It is written to the **unified GLSL convention**, so the repo's shader editor drives it
end to end without any host code:

```bash
just shader-editor landscape_shader/landscape_textures/terrain.glsl
just shader-editor landscape_shader/landscape_textures/terrain.glsl --check
just shader-editor landscape_shader/landscape_textures/terrain.glsl --out .output/t.png --channel all --tiles 2
```

That gives a slider per member of all three uniform blocks, a lit preview of the G-buffer,
a tiled view for checking that a bake joins itself, and a "Save tuning" that writes the
values back onto the default tables at the bottom of the file. The conventions are in
`utils/shader_editor/README.md`. Compiling with `USE_DEFAULTS` bakes from those tables and
reads none of the three bindings, which is how it is checked without a host.

## Dependencies

winit, wgpu and the egui bridges are not on the root Cargo.toml menu; the owner approved
them for this crate on 2026-09-20. egui is held at the house pin 0.31.1.
