---
name: soe
description: The house Standard Operating Environment for this Rust project. Read this before running anything, writing a file to disk, adding a script or recipe, or deciding where output goes. Answers "how does this repo work" without re-deriving it from the tree. Triggers on "how do I build/test/run this", "where does this go", "add a just recipe", "add a script", "why is vet failing", "set up the website", "what are the conventions here".
---

# Standard Operating Environment

Every project in this family is laid out the same way. Learn it once.

## The one gate

`just vet` is what must pass before anything is pushed. It runs, in this order:

| Step              | Checks                                                                |
|-------------------|------------------------------------------------------------------------|
| `vet-editor`      | No unsaved VS Code buffers, so vet cannot pass against a stale tree.   |
| `vet-chars`       | No AI-ism non-ASCII punctuation in tracked rs/py/sh.                   |
| `vet-scripts`     | The justfile and every scripts/*.sh, *.py parse.                       |
| `build`           | `cargo build`.                                                         |
| `test`            | `cargo test`.                                                          |
| `vet-doc`         | Doc tests, plus a rustdoc pass over the workspace.                     |
| `find-new-smells` | In-house smell scan, scoped to files with pending git changes.         |
| `check-last-ci`   | Prints the result of the last CI run. LAST, because the others clear the screen. |

There is exactly one gate. `vet` passing means good to go, so every check the project has is
in that list. Never add a second tier: a check the project grows goes into the `vet`
dependency list, before `check-last-ci`.

CI runs the identical `just vet`. If CI and vet ever check different things the dev box
stops being trustworthy, so a new check goes in `vet`, never straight into the workflow.

## Folder map

A leading dot means "not part of the product". Tracked vs ignored is deliberate:

| Folder            | Tracked | Holds                                                            |
|-------------------|---------|-------------------------------------------------------------------|
| `src/`            | yes     | The code. `lib.rs` declares modules, `main.rs` is a shell.        |
| `scripts/`        | yes     | All shell and python tooling. Always run from the repo root.      |
| `doco/`           | yes     | `about_<subsystem>.md` deep dives, indexed in CLAUDE.md.          |
| `test_data/`      | yes     | Fixtures and inputs for tests and dev utilities.                  |
| `web/`            | yes     | The website source, if there is one.                             |
| `utils/`          | yes     | Standalone dev tools, each its own crate OUTSIDE the workspace.  |
| `.plans/`         | yes     | The owner's notes. Off limits to you unless you are invited in.   |
| `.claude/skills/` | yes     | Project skills. The rest of `.claude/` is personal and ignored.   |
| `.output/`        | NO      | Dump folder for script/util byproducts: query results, stats, graphs, run logs. |
| `.test_output/`   | NO      | Per-run artefacts from test harnesses.                           |
| `deploy/`         | NO      | The staged website, rebuilt from scratch every time.             |
| `.venv/`          | NO      | The project python. Created by `just setup`.                     |

**Anything a script or recipe writes as a byproduct goes to `.output/`.** Never the repo
root, never the cwd, never a new folder invented for the occasion. `just setup` creates it
and scripts assume it exists rather than mkdir-ing it.

## Where things go

- **A command you would type twice** -> a `just` recipe, with a comment above it saying
  what it does and showing an example invocation. A recipe with no comment is a recipe
  nobody else can use.
- **More than about ten lines of logic** -> a script in `scripts/`, with a one-line recipe
  pointing at it. CI then calls the script directly and the logic stays testable.
- **Shell helpers** -> `scripts/_lib.sh` (`log`, `warn`, `err`, `die`, `run`, `need`,
  `banner`, `repo_root`, `cd_repo_root`, `venv_python`, `load_env`). Source it, do not
  re-implement it.
- **Python** -> always through `.venv`. New non-stdlib import means a line in
  `scripts/requirements.txt`. A standalone script starts with
  `from _venv_bootstrap import run_in_venv; run_in_venv()` so it re-execs itself into the
  venv rather than failing on a missing import.
- **A new tool the project needs** -> add it to `just setup`. Never install ad-hoc; a
  machine that was set up by following the recipe must end up able to build.
- **A dev tool** (an editor, a viewer, a harness with a window) -> `utils/<name>/`, its own
  crate with an empty `[workspace]` table so it stays out of the project's dependency graph,
  plus a recipe that runs it. Utils are support code: `unwrap()` and printing to stdout are
  fine there, and the smell scanner knows it.
- **A crate for some job** -> the commented menu in `Cargo.toml` first. Those are already in
  use and understood, at versions that match, so nothing new gets dragged in. A crate that is
  not on the menu is a decision requiring approval.
- **Knowledge a future task will need** -> `doco/about_<subsystem>.md`, linked from CLAUDE.md.
- **A plan, a design note, a status update** -> nowhere, by default. `.plans/` and `todo.md`
  are the owner's personal space, not a filing cabinet for your work. Do not read them or
  write to them unless invited by name; if you are asked for a plan and not told where it
  goes, put it in the reply.

## Code rules that bite

Full text in `code_standard.md`. The ones that come up constantly:

- No `unwrap()`, no `panic!`, no `print!`/`println!`/`dbg!` in production code.
  Production is anything that is not a unit test and not under `bin/`, `tests/`,
  `benches/`, `examples/`.
- **No silent fallbacks.** Code never pretends it did its job. Best-effort returns are for
  when that is inherent in the problem, not when it avoids plumbing. Log a warning at
  minimum.
- **Comments are one line** unless there is a real reason. Multi-line comments cry wolf.
  No backticks in comments; a run of three or more backticked comment lines is flagged as
  AI bloat by the smell scanner.
- **Never delete an existing comment** unless the code it describes is going too.
- Comment tags are `// TAG: text`, from: TODO, FIXME, WIP, KLUDGE, HACK, WHERE_IS, NOTE,
  ISSUE, POLICY.
- Area breaks are `// ---...` finishing on col 120. Code at the top of the file, three
  consecutive area breaks, then tests at the bottom.
- **`lib.rs` and `mod.rs` are scaffolding only**: module declarations and re-exports. No
  definitions, no tests. Code goes in a named module beside them.
- **ASCII only.** No em-dash, en-dash, ellipsis, curly quotes or NBSP in code, comments,
  commit messages or chat. No emoji in script files. `just vet-chars` enforces it.
- A new crate dependency is a major decision. Flag it, do not just add it.

## Utils

`utils/` holds dev tools that travel with the SOE rather than with a project. Each is a crate
outside the workspace: own lockfile, own target, none of its dependencies in the product.
`cargo build` at the root does not touch them, `just vet` does not wait on them, and CI does
not build them.

`shader_editor` is the one that ships: point it at a `.glsl` and it gives you a hot-reloading
preview with a control for every uniform, generated from the uniform's own comment metadata.
It also runs headless (`--check` / `--out foo.png`), which is how it is usable from a script
or by an agent. Full grammar in `utils/shader_editor/README.md`.

It reads two conventions and works out which from the file:

- **paired**: `foo.glsl` plus a `foo.vert` of the same stem, GLSL 330, flat uniforms, one
  output, drawn by macroquad. For a 2D effect or a post pass.
- **unified**: one file carrying both stages behind `#ifdef` guards, GLSL 450 as naga reads
  it, `layout(std140, binding = N)` uniform blocks, and several fragment outputs whose lanes
  declare their PBR role (`// !(pbr=roughness:r)`). Compiled by naga and drawn offscreen by
  wgpu, previewed lit or one channel at a time. For a material, a G-buffer or a texture bake.

Note for this project: `landscape_shader/` is wgpu with WGSL strings inside the rust, so the
editor does not drive those. `landscape_shader/landscape_textures/terrain.glsl` is GLSL 450
and is the unified convention's worked example.


## Website (only some projects have one)

Cloudflare Pages, always, and always via a separate `<project>_site` repo: this repo builds
a folder, `deploy_site.yml` pushes it to a branch there (prod/dev/staging), Pages deploys
from that. Deploy is manual (`workflow_dispatch`), never on push.

- `just website-images` generates every derived image referenced by an `/auto_<folder>/`
  URL in `web/*.html`. The filename is the spec: `/auto_images/hero_640w_70q.webp` means
  "take `images/hero.*`, 640px wide, quality 70, write it as webp". Source format need not
  match destination format. Nothing derived is checked in.
- `just website-chores` runs every derived-asset chore. Each chore is mtime-guarded, and
  everything it writes is listed in both `.gitignore` and its `--clean` list.
- `just build-website` stages `deploy/`; `just launch-website` serves that same folder, so
  local preview and production are the same bytes.

## Getting oriented in an unfamiliar project here

1. `just --list` tells you what the project can do. The recipe comments are the docs.
2. `CLAUDE.md` has the layout, the conventions and the index of `doco/`.
3. `doco/about_*.md` is the handover material for whichever subsystem you are touching.
4. `git log` and the recipe comments, for how the place actually behaves.

`.plans/` and `todo.md` are not on that list on purpose. They are the owner's, and reading
them uninvited is going through someone's desk.
