#!/usr/bin/env python3
"""Regenerate test_data/xbr2x/: a synthetic RGBA image and what xBRjs makes of it.

sh_formats ports the 2x kernel of xBRjs (reversing/web/vendor/xBRjs.umd.js, MIT) and its
test holds the port to this output byte for byte, so a port that gets the blending wrong
fails rather than merely looking a bit different. Needs node on the path.

    python3 scripts/gen_xbr_fixture.py
"""
import os
import struct
import subprocess
import sys

WIDTH, HEIGHT = 16, 12
OUT = "test_data/xbr2x"
VENDOR = "reversing/web/vendor/xBRjs.umd.js"


def pixel(x, y):
    """Shapes that exercise the kernel: a diagonal edge, a blob with a hole, two colours a
    hair apart, and transparent surround. Deterministic; nothing random."""
    if x + y < 6:
        return (200, 40, 40, 255)
    if (x - 10) ** 2 + (y - 6) ** 2 < 12:
        if (x - 10) ** 2 + (y - 6) ** 2 < 2:
            return (0, 0, 0, 0)
        return (40, 200, 60, 255) if (x + y) % 5 else (44, 202, 58, 255)
    if y > 9 and x % 3 == 0:
        return (60, 60, 220, 128)
    if 3 <= y <= 4 and 8 <= x <= 12:
        return (250, 250, 250, 255)
    return (0, 0, 0, 0)


def main():
    if not os.path.exists(VENDOR):
        sys.exit(f"{VENDOR} is missing; run from the repo root")
    os.makedirs(OUT, exist_ok=True)
    src = bytearray()
    for y in range(HEIGHT):
        for x in range(WIDTH):
            src += struct.pack("4B", *pixel(x, y))
    with open(f"{OUT}/input.rgba", "wb") as fh:
        fh.write(src)
    driver = f"""
        const fs = require("fs");
        const vm = require("vm");
        const [vendor, input, output] = process.argv.slice(1);
        // The UMD wrapper names window before it tests for CommonJS; give it one, untouched.
        const sandbox = {{ window: {{}}, module: {{ exports: {{}} }}, exports: {{}} }};
        vm.runInNewContext(fs.readFileSync(vendor, "utf8"), sandbox);
        const xbr = sandbox.module.exports;
        const src = fs.readFileSync(input);
        const px = new Uint32Array(src.buffer, src.byteOffset, src.length / 4);
        const out = xbr.xbr2x(px, {WIDTH}, {HEIGHT}, {{ scaleAlpha: true }});
        fs.writeFileSync(output, Buffer.from(out.buffer));
    """
    subprocess.run(["node", "-e", driver, os.path.abspath(VENDOR), f"{OUT}/input.rgba",
                    f"{OUT}/expected.rgba"], check=True)
    with open(f"{OUT}/README.md", "w") as fh:
        fh.write(f"""# xBR 2x reference

input.rgba is a {WIDTH}x{HEIGHT} RGBA image, expected.rgba is {WIDTH * 2}x{HEIGHT * 2}: what
xBRjs (reversing/web/vendor/xBRjs.umd.js, commit f8b3ab8, MIT) produces for it with
scaleAlpha on, which is how the asset browser calls it. Both are written by
scripts/gen_xbr_fixture.py; sh_formats' xBR test requires its port to match expected.rgba
exactly. Regenerate only if the vendored library changes.
""")
    print(f"{OUT}: {len(src)} input bytes, {os.path.getsize(f'{OUT}/expected.rgba')} expected bytes")


if __name__ == "__main__":
    main()
