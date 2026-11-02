#version 330
#ifdef GL_ES
    precision mediump float;
    precision highp int;
#endif

// Every metadata form the editor understands, in one file. Run it with:
//   just shader-editor utils/shader_editor/examples/demo.glsl

in vec2 v_uv;
out vec4 fragColor;                 // !(format=RGBA)

uniform vec2  u_resolution_px;      // !(format=resolution, default=[512,288]) preview size, bound live
uniform float u_time;               // !(format=TIME, max=8.0) sawtooth seconds, Play/Pause/scrub in the editor
uniform float u_percent;            // !(min=0.0, max=1.0, anim=4.0) auto-lerps min to max every 4s
uniform float u_percent_integral;   // !(format=integral, what=u_percent) accumulates u_percent * dt
uniform vec4  u_fill_col;           // !(format=RGBA, default=#1e3a5fff) background
uniform vec3  u_bar_col;            // !(format=RGB, default=#f0a030) progress bar
uniform float u_bar_h;              // !(default=0.12, min=0.02, max=0.5) bar height, fraction of the frame
uniform sampler2D u_label;          // !(default=text:"SOE", size=[256,64]) generated text stand-in

// --------------------------------------------------------------------------------------------------------------------
// Tuning block: the #ifdef arm drives editor sliders, the #else arm holds what the project
// compiles. "Save tuning" writes the live slider values onto those consts.
#ifdef EDITOR   // !(group="wobble")
uniform float t_wobble_amp;         // !(default=0.03, min=0.0, max=0.2)
uniform float t_wobble_freq;        // !(default=6.0, min=0.5, max=20.0)
#else
const float t_wobble_amp  = 0.03;
const float t_wobble_freq = 6.0;
#endif

// --------------------------------------------------------------------------------------------------------------------

void main() {
    vec2 uv = v_uv;

    // A bar across the frame, wobbling, filling with u_percent, with a label riding on it.
    float wobble = t_wobble_amp * sin(uv.x * t_wobble_freq + u_time * 2.0);
    float bar_lo = 0.5 - u_bar_h * 0.5 + wobble;
    float bar_hi = bar_lo + u_bar_h;
    float in_bar = step(bar_lo, uv.y) * step(uv.y, bar_hi);
    float filled = step(uv.x, u_percent);

    vec3 col = u_fill_col.rgb;
    col = mix(col, u_bar_col * 0.35, in_bar);
    col = mix(col, u_bar_col, in_bar * filled);

    // The integral drives a slow drift, so it is visible that it accumulates rather than tracks.
    float drift = fract(u_percent_integral * 0.25);
    col += 0.05 * drift;

    // Label sits in the top-left corner; the stand-in is transparent outside the glyphs.
    vec2 label_uv = (uv - vec2(0.04, 0.80)) / vec2(0.30, 0.12);
    if (all(greaterThanEqual(label_uv, vec2(0.0))) && all(lessThanEqual(label_uv, vec2(1.0)))) {
        vec4 label = texture(u_label, vec2(label_uv.x, 1.0 - label_uv.y));
        col = mix(col, label.rgb, label.a);
    }

    fragColor = vec4(col, u_fill_col.a);
}
