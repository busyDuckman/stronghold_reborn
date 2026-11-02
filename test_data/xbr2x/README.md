# xBR 2x reference

input.rgba is a 16x12 RGBA image, expected.rgba is 32x24: what
xBRjs (reversing/web/vendor/xBRjs.umd.js, commit f8b3ab8, MIT) produces for it with
scaleAlpha on, which is how the asset browser calls it. Both are written by
scripts/gen_xbr_fixture.py; sh_formats' xBR test requires its port to match expected.rgba
exactly. Regenerate only if the vendored library changes.
