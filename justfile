# justfile
# Stronghold task runner.
#
# `just` is the only task runner; there is no make. Anything you would otherwise type twice
# belongs here, and anything longer than about ten lines belongs in scripts/ with a one-line
# recipe pointing at it, so CI can call the script directly and the logic stays testable.
#
# Comment style, because `just --list` only shows the LAST comment line above a recipe:
#   - prose block first (the why, the gotchas, example invocations)
#   - blank line
#   - ONE summary line, directly above the recipe
# A recipe with no summary line is a recipe nobody else can use.

set shell := ["bash", "-cu"]
set windows-shell := ["powershell.exe", "-NoLogo", "-Command"]

# The project's python. Every python recipe goes through the venv explicitly, so system and
# --user packages are never touched. Created by `just setup`.
PYTHON := ".venv/bin/python3"

# List every recipe (this is what you get for a bare `just`).
default:
    @just --list

# -------------------------------------------------------
# setup

# Every tool the project needs is installed here. Never install ad-hoc: a machine set up by
# following this recipe must end up able to build. Uncomment the lines as the project grows
# into them.

# First-time setup on a fresh clone or a new machine.
setup:
    rustup toolchain install                      # honours rust-toolchain.toml
    cargo install --locked cargo-nextest
    python3 -m venv .venv
    {{PYTHON}} -m pip install --upgrade pip
    {{PYTHON}} -m pip install -r scripts/requirements.txt
    # Dump folder for script/util byproducts. Gitignored; recipes assume it exists.
    mkdir -p .output
    # rustup target add wasm32-unknown-unknown
    # cargo install wasm-pack
    # git lfs install --local && git lfs pull
    # npm --prefix web/website install --no-audit --no-fund
    # sudo apt-get install -y jq ffmpeg libvips-tools

# -------------------------------------------------------
# build

# The root is a workspace (the app plus sh_formats), so the build and test recipes say
# --workspace; a bare cargo build here would leave the library crates out of the gate.

# Debug build.
build:
    clear; cargo build --workspace

# Type-check without producing a binary.
check:
    clear; cargo check --workspace

# Optimised build.
build-release:
    clear; cargo build --workspace --release

# Build and run. Args pass through: just run --help
run *ARGS:
    cargo run --bin stronghold -- {{ARGS}}

# Build and run, optimised.
run-release *ARGS:
    cargo run --release --bin stronghold -- {{ARGS}}

# -------------------------------------------------------
# test

# Run the unit tests.
test:
    clear; cargo test --workspace

# Run the unit tests through nextest (faster, better output).
nextest:
    clear; cargo nextest run --workspace

# By convention `_dev` is a chunk of code kept around to poke at something under
# development. Warnings are muted so the output is just what the test printed.

# Run the _dev scratch test with output.
[unix]
dev:
    RUSTFLAGS="-A warnings" cargo test _dev -- --nocapture --ignored

# Run the _dev scratch test with output.
[windows]
dev:
    $env:RUSTFLAGS="-A warnings"; cargo test _dev -- --nocapture --ignored

# -------------------------------------------------------
# vet
#
# THE gate. There is only one. `vet` passing means good to go, so every check the project
# has lives in this list. No fast tier, no full tier: a second tier just means the first one
# stops meaning anything.
#
# The order reads: cheap static checks, the build, the tests, the smell scan, then the report
# on the last CI run. check-last-ci stays LAST because the earlier recipes clear the screen.
#
# CI runs this identical command. If vet and CI ever check different things the dev box stops
# being trustworthy, so a new check goes here, never straight into the workflow.
#
# A check the project grows later (vet-wasm, an asset-manifest check, whatever it is) gets
# added to the dependency list below, before check-last-ci. Nowhere else.

# THE gate. Everything the project checks. Must pass before anything is pushed.
vet: vet-chars vet-scripts build test vet-doc find-new-smells check-last-ci
    @just banner "vet passed" green

# Without this a long vet can pass against a stale working tree while the real edit sits in
# an unsaved editor buffer.

# Scan tracked rs/py/sh for AI-ism non-ASCII punctuation. Args override the extension list.
vet-chars *EXTS:
    bash scripts/vet_chars.sh {{EXTS}}

# Parse-check the justfile and every .sh / .py under scripts/.
vet-scripts:
    bash scripts/vet_scripts.sh

# Doc tests are real tests and rustdoc is a real compiler: this catches examples that stopped
# compiling and intra-doc links that stopped resolving. Neither shows up in `cargo test`.

# Run the doc tests and build the docs.
vet-doc:
    cargo test --workspace --doc
    cargo doc --workspace --no-deps

# Does the wasm target still build? Uncomment it in `vet` once a wasm target exists.
vet-wasm:
    cargo build --target wasm32-unknown-unknown

# Scan every .rs for in-house smells: unwrap in production, stray printing, sentinel uuids,
# AI comment bloat. Project-specific smells are added in the script.

# Full in-house code smell sweep over the whole tree.
find-smells:
    {{PYTHON}} scripts/check_for_shit_code.py

# Same scan, scoped to files with pending git changes (staged, unstaged and new untracked),
# so you clean up what you are working on rather than the whole repo. This is the one in
# `vet`; the full sweep is a deliberate act.

# Code smell sweep over files with pending git changes only.
find-new-smells:
    {{PYTHON}} scripts/check_for_shit_code.py --git-pending

# Print the result of the last CI run. Skips quietly if .upstash.env is absent.
check-last-ci:
    bash scripts/git/check_ci.sh

# The public mirror: CI pushes the public part of every push to main to busyDuckman/stronghold_reborn. What is held
# back is .gitignore plus .extra_gitignore.public. This shows what would go up from HEAD, and pushes nothing.

# Dry run of the public mirror: what it would hold back and publish from HEAD.
public-preview:
    bash scripts/git/publish_public.sh

# -------------------------------------------------------
# docs

# Build the API docs.
doco:
    clear; cargo doc --workspace --no-deps

# Build the API docs and open them.
open-doco:
    clear; cargo doc --workspace --no-deps --open


# -------------------------------------------------------
# helpers

# Body lives in scripts/_lib.sh so scripts can call it too. Colors: red, orange, green, blue.

# Print a centred, coloured banner.
banner message color="orange":
    bash scripts/banner.sh "{{message}}" "{{color}}"

# List open issues assigned to you. Args pass through: just issues --label bug
issues *ARGS:
    gh issue list --assignee "@me" --limit 100 {{ARGS}}

# Wipe the derived-output folders. Never touches anything git tracks.
clean:
    cargo clean
    rm -rf .output/* deploy .website
    rm -rf utils/*/target
    mkdir -p .output

# -------------------------------------------------------
# sh_formats

# The game's file formats live in sh_formats/ (doco/about_sh_formats.md). dat_util is that
# crate's test CLI: list, info, raw extraction and PNG/WAV export from STRONG.DAT by index,
# range or source filename. Output lands in .output/dat_util/. The names come from a .tsv beside the
# .dat, or from --names; the reversing pass's segmentation.tsv and dat_names.tsv both load.
#   just dat-util info 36 hourgls4.lbm:3
#   just dat-util --names reversing/work/names/segmentation.tsv extract build\\geninn01.lbm
#   just dat-util extract --all
#   just dat-util export 1170-1199 --strip --filter xbr2x

# Poke at STRONG.DAT. Args pass through: just dat-util list --type pal
dat-util *ARGS:
    cargo run -q -p sh_formats --bin dat_util -- {{ARGS}}

# ungrab takes STRONG.DAT apart into the source tree the game's own packer, STRONG.EXE grab,
# builds it from: every file grab opens at its DOS path, plus ungrab.txt (how to rebuild in
# DOSBox, the reserved colours) and ungrab.tsv (every entry's rectangle). OUT must be new or
# empty. A container grab rebuilt from an edited tree needs --accept-layout. The proof that grab
# over the tree gives STRONG.DAT back is reversing/scripts/ungrab_check.py (doco/about_sh_formats.md).
#   just ungrab .output/ungrab
#   just ungrab .output/ungrab_mod path/to/rebuilt/STRONG.DAT --accept-layout

# Write the source tree STRONG.EXE grab rebuilds STRONG.DAT from: just ungrab .output/ungrab
ungrab OUT DAT="achieve/archive.org/en_cdrom/unpacked/STRONG.DAT" *ARGS:
    cargo run -q --release -p sh_formats --bin ungrab -- "{{DAT}}" "{{OUT}}" {{ARGS}}

# -------------------------------------------------------
# landscape_shader

# The terrain renderer test bed in landscape_shader/ is its own crate with its own justfile,
# outside the workspace and off the vet path. This is the one hook into it from the root.

# Run the landscape renderer test bed. Args pass through: just landscape --release
landscape *ARGS:
    cargo run --manifest-path landscape_shader/Cargo.toml {{ARGS}}

# -------------------------------------------------------
# utils
#
# Standalone dev tools under utils/, each its own crate outside the workspace. They are not
# built by `cargo build`, not covered by `just vet`, and not built in CI. See utils/README.md.

# Live GLSL editor: hot reload, generated uniform controls, tuning values written back.
# It reads two conventions and works out which from the file itself:
#   paired  - foo.glsl + foo.vert, GLSL 330, one output, drawn by macroquad
#   unified - one file, GLSL 450, uniform blocks and a G-buffer, compiled by naga and drawn
#             by wgpu, previewed lit or one PBR channel at a time
#   just shader-editor utils/shader_editor/examples/demo.glsl          the paired example
#   just shader-editor utils/shader_editor/examples/demo_pbr.glsl      the unified example
#   just shader-editor path/to/foo.glsl --check
#   just shader-editor path/to/foo.glsl --out .output/foo.png --size 512x128 --time 2.5
#   just shader-editor path/to/mat.glsl --out .output/m.png --channel all --tiles 2

# Not a release build on purpose: this is a dev tool, so the first-run build time is what you
# feel, and the shader itself runs on the GPU either way. Its dev profile is opt-level 1,
# because the wgpu stack is painful at 0.
# VS Code: the "Shader Editor (active GLSL)" task runs this on the file you are looking at.

# Open a shader in the live editor. Args pass through (see utils/shader_editor/README.md).
shader-editor *ARGS:
    cargo run --manifest-path utils/shader_editor/Cargo.toml -- {{ARGS}}
