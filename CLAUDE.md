# Stronghold - Claude Code Instructions

## Project Overview

An engine remake of Stronghold (Stormfront Studios / SSI, 1993, MS-DOS), heading for a modern
Rust engine that runs on the original game's data. The goal is a working port, not a full
decompile; scope and the rigour each part gets are in `reversing/notes/about_port_scope.md`.

- `reversing/`: the reversing pass. STRONG.EXE is recovered function by function into Open
  Watcom C, and each function is checked against the original running under the Unicorn
  emulator. Agents do most of this work; see the `reverser` and `reverser_bulk` agents.
- `sh_formats/`: Rust readers for the game's files (STRONG.DAT, palettes, sprites, sounds) and
  the ungrab.
- `landscape_shader/`: a terrain renderer test bed (wgpu, egui) with a port of the original map
  generator. It may become the renderer or be thrown away.
- The Rust port of the recovered C is still to come.

This repo is private. On every push to `main`, `.github/workflows/public_mirror.yml` mirrors
everything except what `.gitignore` plus `.extra_gitignore.public` hold back to the public
repo `busyDuckman/stronghold_reborn` (GPL-2.0-only). The recovered C never goes public.

## Standard Operating Environment

This project follows the house SOE. **Read `.claude/skills/soe/SKILL.md` before running
anything, writing a file, or adding a recipe.** It covers the folder conventions, the `vet`
gate, where output goes, and where new tooling belongs. Do not re-derive them from the tree.

The short version:

- `just vet` is the gate. CI runs the same command.
- `just --list` is the command surface. Recipe comments are the documentation.
- The reversing pass keeps its own `reversing/justfile`. Run those from `reversing/`;
  the root justfile knows nothing about them.
- Byproducts go to `.output/`. Subsystem docs go to `doco/`.
- `.plans/` and `todo.md` are the owner's personal space. Do not read or write either
  unless invited by name.
- Python goes through `.venv`; new deps go in `scripts/requirements.txt`.
- ASCII only, everywhere. No em-dashes, curly quotes, ellipses or NBSP.

## Code standard

`code_standard.md` in the repo root. It is not advisory. The parts that bite most often: no
`unwrap()` or `panic!` or printing in production code, no silent fallbacks, one-line
comments, never delete an existing comment, area breaks at col 120 with code above and tests
below three of them.

## Deep-dive docs

Subsystem details live in `doco/`. Read the relevant file when a task touches that area:

<!-- Add one line per doc as they are written. A doc that is not linked here does not exist. -->
- `doco/about_sh_formats.md` - the sh_formats crate: STRONG.DAT access by index or filename,
  entry classification, the palette rules with their evidence, and the media layer (sprites,
  animations, sounds, PNG/WAV export, xBR).

## Layout

```
stronghold/
|-- src/
|   |-- lib.rs     # scaffolding only: module declarations and re-exports
|   |-- prelude.rs # the everyday names, re-exported
|   |-- app_info.rs# a module: code at the top, its own tests at the bottom
|   \-- main.rs    # a shell around the lib
|-- sh_formats/    # readers for the game's file formats: a workspace member, on the vet path
|   |-- src/file_format.rs        # the generic reader interface; file_type_detector.rs names a payload
|   |-- src/formats/              # one handler per payload format: image, palette, audio
|   |-- src/dat/                  # the archive: DatFile, the names file, the palette rules
|   |-- src/media/                # for modern use: Image, Sprite, Animation, AudioClip, export, filters
|   \-- src/bin/dat_util.rs       # the test CLI: list, info, extract raw payloads
|-- doco/           # about_<subsystem>.md deep dives, indexed above
|-- test_data/      # fixtures and inputs for tests and dev utilities
|-- web/            # website source (delete if this project has no site)
|-- utils/          # standalone dev tools, each a crate OUTSIDE the workspace
|   \-- shader_editor/   # live GLSL editor, paired and unified; `just shader-editor <file.glsl>`
|-- scripts/        # all shell/python tooling; always run from the repo root
|   |-- _lib.sh              # shared shell helpers: log/err/die/run/need/banner/repo_root
|   |-- _venv_bootstrap.py   # re-exec a python script through .venv
|   |-- check_for_shit_code.py   # in-house smell scanner (just find-smells)
|   |-- vet_*.sh             # the static half of the vet gate
|   |-- git/                 # CI beacon, artefact housekeeping
|   \-- web/                 # website chores, auto images, galleries, deploy folder
|-- justfile        # every build/run/test/deploy command
|-- reversing/      # the DOS reversing pass: its own justfile, venv and scripts
|-- landscape_shader/ # terrain renderer test bed: its own crate and justfile, off the vet path
\-- .plans/         # the owner's notes (tracked, and off limits unless invited)
```

## Build & Run

Use `just`, not make:

```bash
just setup          # first-time setup on a fresh clone (toolchain, tools, venv, .output)
just build          # debug build
just build-release  # release build
just run            # build + run
just test           # cargo test
just nextest        # same, faster
just vet            # THE gate: must pass before pushing
```

## Key Conventions

<!-- Fill in as the project grows one. The template has none of these on purpose; they are
     the things a newcomer gets wrong. Examples of what belongs here:

### Time
     Which clock is authoritative, and what is banned.

### Logging
     The log macros, what each level means, any tagged-logging side effects.

### Error Handling
     The project's Result alias, how errors are converted, what the UI does with them.

### Localisation
     The translation macros and the file format rules.
-->

## Architecture Notes

<!-- The invariants that are expensive to rediscover and easy to break. Which crate must
     stay GUI-free, which module must not import which, where the state machine lives,
     what is driven per frame. -->

## Key Files

| File | Purpose |
|------|---------|
| `justfile` | All build/run commands |
| `reversing/justfile` | The reversing pass's own commands, run from `reversing/` |
| `utils/shader_editor/README.md` | The live GLSL editor: the two shader conventions, hint grammar, PBR roles, tuning, headless mode |
| `code_standard.md` | How code is written here |
| `.claude/skills/soe/SKILL.md` | The environment: folders, gates, where things go |
| `.extra_gitignore.public` | What the public mirror holds back on top of `.gitignore`; to keep something private, add it here (`just public-preview` to check) |
