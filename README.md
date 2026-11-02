# Stronghold Reborn

![Rust](https://img.shields.io/badge/rust-1.95-orange?logo=rust)
![Original](https://img.shields.io/badge/original-MS--DOS%201993-blue?logo=dosbox)
![Reversing](https://img.shields.io/badge/reversing-Ghidra%20%7C%20Open%20Watcom%20%7C%20Unicorn-8a2be2)
[![CI](https://github.com/busyDuckman/stronghold_reborn/actions/workflows/ci.yml/badge.svg)](https://github.com/busyDuckman/stronghold_reborn/actions/workflows/ci.yml)


This is an engine remake for the DOS video game [Stronghold](https://en.wikipedia.org/wiki/Stronghold_(1993_video_game)) (1993, SSI) by Stormfront Studios.  


![Widescreen test](doco/screenshots/widescreen_test.png)

While I intend to release a faithful build, using Rust for Win/Linux/Mac, it's not why I am here.

I feel Stronghold was a significant early entry into RTS games. It made some gameplay decisions that did not stick. The genre was very soon defined by Dune II (1992), Warcraft (1994), and Command & Conquer (1995), which went another way in terms of how an RTS game could be played. AFAIK, all mainstream RTS games evolved from these conventions.

I want to step back and say, what if Stronghold was the more influential of the titles in the early 1990s. How would modern RTS games feel if they evolved from those concepts? That's been a question that has nagged at me for a while, so I want to create something that answers that.

**Note:** This repo is a public mirror of the (private) development repo, sans the reversing work (which I can't distribute). No development is done in this repo.

## Reversing Effort

After poking around a bit I discovered some previous work: [DeSt](https://github.com/NancySadkov/dest) by NancySadkov, who cracked the STRONG.DAT asset format and the overlaid STRONG.EXE.
Big thanks; their ungrab and mzap tools were a good leg up.

The original code was built with Borland C++, and the norm would be to decompile back to that... but I had an idea. 

**Reverse directly to Open Watcom**, and check each function by running the original in the [unicorn](https://github.com/unicorn-engine/unicorn) emulator and fuzzing the Watcom version against it, comparing registers, memory and data segment. I know a similar check worked well for OpenRCT2 (TestPaint). So I set up some tooling to do so. It would be slower, but I would not be tied to Borland C++ 3.0, which can't legally be redistributed.

With the harness ready, I got Claude onto the reversing task and it went fairly well, if slowly. It's been said that AI can fully reverse anything now. My experience was that there were a number of things it could not get without human help (the game's palettes were a good example, as was reckoning the sprite layout, and a few odd game features).

After I solved those few roadblocks manually, the agentic reversing proceeded in a (mostly) orderly fashion. I just verified things, pointed out a few errors, and left the AI to cook on its own. 
This will likely be the last time I ever do any manual reversing work, and I'm kinda glad to end that journey on Stronghold. 


### Status

| Part | Size | State |
|------|------|-------|
| Simulation (economy, movement, the computer leaders, raising forces, seasons, combat, winning and losing, new world) | ~33k | Core loop done, but it leans on ~30k of rules hiding in the shared code that still need doing (unit vs unit combat, battles on the map, building jobs, the census). |
| The save format | 3.8k | Done. Plus a Python reader and writer. |
| Screens and behaviour | ~66k | Done, except the asset packer (`grab`, which the ungrab covers). What a screen computes is recovered; how it paints is left to the port. Animation of sprites is still not solved.|
| Shared services | ~70k | Partly done. Turns out a lot of the real game rules live in here, so that's next. The rest is dialogs and menus, which the port can just do its own way. |
| Drawing, sound, devices | ~54k | Not going to attempt. Just create new code for the port. |

So far 410 functions and about half of the program's code (147k of 293k bytes) are recovered and verified.

### Trivia

1) During decompile it was discovered the game has a "developer mode".

Start the game using "strong debug" 
  - Alt-D: brings up an info screen that shows some sort of mask and some numbers to do with the terrain (haven't figured out the full meaning, but seems almost decipherable from just walking around.)
  - Alt-S: cycles season
  - ALT-B cycle building type (Alt-F is a 10x FFW version), I think it only works for the
    building 2nd to the left.
  - ALT-L raise a character's level (ALT-C, ALT-P something related to characters as well)
  - ALT-K destroy an enemy stronghold, ALT-Z destroy enemy units
  - Alt-G destroy your own stronghold?

2) The game uses D&D look and terms, but AFAIK the underlying combat code is not D&D mechanics (not D20 or saving throws etc).

3) STRONG.EXE is not just the game. The debug keys above are a crude map editor (Alt-F's building changes survive a save), and Stormfront's asset packer is still inside it too. Run `strong grab` and it rebuilds STRONG.DAT from the source art.


To this end a corresponding ungrab was made:

```bash
just ungrab .output/ungrab        # STRONG.DAT -> the 475 source files grab reads, plus ungrab.txt
```

  - Never run `strong grab` in your real game folder: the first thing it does is wipe STRONG.DAT. The full
    rules (reserved colours, animation markers, the font) are in `ungrab.txt` in the output.


4) Some crash errors were internal, e.g. running "strong grab" produced:
  
    Error 0 13.  (Give Cathryn BOTH numbers.)


# Basic world to explore

Currently there is a terrain renderer test bed using a working port of the original map generator. It lets you walk around in a 'widescreen' adaptation of the original graphics (as per the image at the top of the readme).



Run it from the repo root:

  ```bash
  just landscape              # debug build
  just landscape --release    # optimised build
  ```

It's written in Rust, with wgpu for the rendering, and egui for the menus. The terrain is a mesh built from the map's corner heights. It's in its own crate, because it may end up being the game's renderer or it may get thrown away. **You need the original game to launch this demo.**

# Future Work

## Modern Port

Let's port the reversed C++ code to Rust and get a version running, distributing only the engine; you still need to acquire Stronghold.

What has been done (so far):

- `sh_formats`: reads the game's files. STRONG.DAT by index or by name, the palettes, sprites, animations and sounds, with export to PNG and WAV (and an xBR filter to upscale the sprites). The ungrab lives here too.

## New Engine

I'm still chewing the cud on this. I also saw a few good ideas in this [reddit thread](https://www.reddit.com/r/dosgaming/comments/1wlk56l/engine_rewrite_stronghold_stormfront_studios_ssi/). 



# License

As per similar projects, the recovered stronghold code is not included in this repo (it's not mine to distribute).

The code in this repo is licensed under the GNU General Public License v2.0 (GPL-2.0-only); see [LICENSE](LICENSE). The game itself is not included; you need your own copy of Stronghold.



