# utils/

Standalone dev tools that travel with the SOE rather than with any one project.

Each subfolder is its own crate, **excluded from the workspace**, with its own `Cargo.lock`
and `target/`. That is deliberate: a shader editor needs macroquad, egui, a GL context and,
these days, wgpu as well, and none of that belongs in the dependency graph of a project that
just wants to build. A util can drag in whatever it needs without any of it reaching the
product, and `just vet` never waits on it.

The consequences of being out of the workspace, so they are not a surprise:

- `cargo build` at the repo root does not build a util. `just <util>` does.
- A util has its own lockfile. It is tracked, because a tool that stops compiling when a
  dependency does a minor release is a tool you will not fix at the moment you need it.
- CI does not build utils. They are dev tools; a broken one costs you an afternoon, not a
  release.

Utils are support code, not production code: `unwrap()` and printing to stdout are fine in
here, and the smell scanner knows it (`/utils/` is in its support-path list). Everything
else in `code_standard.md` still applies.

## What lives here

| Util | What it does |
|------|--------------|
| [`shader_editor`](shader_editor/) | Live GLSL editor: hot reload, uniform controls generated from comment metadata, tuning values written back to the file, and a headless mode for scripted renders and checks. Reads two conventions, paired (GLSL 330, macroquad) and unified (GLSL 450, naga and wgpu, a G-buffer of PBR channels with a lit preview). |

Try it: `just shader-editor utils/shader_editor/examples/demo.glsl` for the paired one,
`just shader-editor utils/shader_editor/examples/demo_pbr.glsl` for the unified one.

## Adding one

Copy the shape of `shader_editor/`: a crate with `[workspace]` empty in its `Cargo.toml`
(that is what excludes it), a `README.md` saying what it is for, and a recipe in the project
justfile that runs it. Delete a util the project does not need, and drop its recipe from the
justfile and its entries from `.vscode/tasks.json` while you are there. It costs nothing to
take one back off the SOE later.
