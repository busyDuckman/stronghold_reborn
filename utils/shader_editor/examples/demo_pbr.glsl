#version 450
// Every part of the unified convention, in one small file. Run it with:
//   just shader-editor utils/shader_editor/examples/demo_pbr.glsl
//
// A brick material: albedo, a normal map with the height in its alpha, and a surface target
// carrying roughness, metallic and ambient occlusion. That is a G-buffer, so the editor lights
// it rather than showing you three pictures of noise.
//
// There is no vertex stage here on purpose. A shader that only wants a fullscreen pass can
// leave it out and the editor supplies one, with v = 0 on the first row of the target. Write
// your own (see landscape_shader/landscape_textures/terrain.glsl) when the host has one.

// -----------------------------------------------------------------------------------------------------------------------
// uniforms: one block, holding one struct plus the two members the editor binds for you

struct Brick {
    vec3  brick_colour;   // !(format=RGB) the face
    float brick_rough;    // !(min=0, max=1)
    vec3  mortar_colour;  // !(format=RGB) the gaps
    float mortar_rough;   // !(min=0, max=1)
    vec2  courses;        // !(lanes=[across, down], min=1, max=16, step=1) bricks per tile edge
    float mortar_width;   // !(min=0.005, max=0.2) as a fraction of one brick
    float bevel;          // !(min=0.001, max=0.2) how rounded the brick edge is
    float variation;      // !(min=0, max=1) lightness spread from brick to brick
    float grain;          // !(min=0, max=1) speckle in the face
    float relief;         // !(min=0, max=1) how far the face stands above the mortar
    float metallic;       // !(min=0, max=1) bricks are not metal; the slider is here to prove the lane works
};

layout(std140, binding = 0) uniform Material {
    Brick brick;
    vec2  res_px;         // !(format=resolution) the bake size, bound live
    float texel;          // !(format=texel) one pixel in uv, the step the normal is taken over
} u_mat;

// -----------------------------------------------------------------------------------------------------------------------
// outputs: what each lane means, so the lit view knows how to shade it

layout(location = 0) out vec4 o_albedo;    // !(pbr=albedo:rgb) colour, a = 1
layout(location = 1) out vec4 o_normal;    // !(pbr=normal:xyz, pbr=height:w) tangent-space normal packed 0..1
layout(location = 2) out vec4 o_surface;   // !(pbr=roughness:r, pbr=metallic:g, pbr=ao:b)

in vec2 v_uv;

Brick default_brick();

Brick current_brick() {
#ifdef USE_DEFAULTS   // !(toggle="bake from the compiled-in table", default=off)
    return default_brick();
#else
    return u_mat.brick;
#endif
}

// -----------------------------------------------------------------------------------------------------------------------
// the material

// An integer hash, not fract(sin(x)): that one bands badly on some drivers and differs between them.
float hash21(vec2 p) {
    uvec2 q = uvec2(ivec2(floor(p)) + 4096);
    uint h = q.x * 374761393u + q.y * 668265263u;
    h = (h ^ (h >> 13u)) * 1274126177u;
    return float(h ^ (h >> 16u)) * (1.0 / 4294967295.0);
}

// Which brick the point is in, and where inside it. Odd courses are offset by half a brick,
// and the whole thing is periodic in courses, so the tile joins itself.
vec4 brick_at(vec2 uv, vec2 courses) {
    float row = floor(uv.y * courses.y);
    float shifted = uv.x * courses.x + 0.5 * mod(row, 2.0);
    float col = floor(shifted);
    return vec4(fract(shifted), fract(uv.y * courses.y), col, row);
}

// Height of the face at uv: 1 on the brick, 0 in the mortar, bevelled between.
float face_height(vec2 uv, Brick b) {
    vec4 cell = brick_at(uv, b.courses);
    vec2 d = min(cell.xy, vec2(1.0) - cell.xy);
    float edge = min(d.x, d.y);
    float mask = smoothstep(b.mortar_width, b.mortar_width + b.bevel, edge);
    // coarser than the albedo speckle on purpose: per-texel noise in a height map comes out
    // as static in the normal, which tells you nothing about the surface
    float speck = hash21(uv * b.courses * 16.0) * b.grain * 0.2;
    return mix(0.0, 1.0 - speck, mask) * b.relief;
}

void main() {
    Brick b = current_brick();
    vec2 uv = v_uv;

    vec4 cell = brick_at(uv, b.courses);
    vec2 d = min(cell.xy, vec2(1.0) - cell.xy);
    float mask = smoothstep(b.mortar_width, b.mortar_width + b.bevel, min(d.x, d.y));

    // one lightness per brick, so a wall does not read as a single flat colour
    float tint = (hash21(cell.zw) - 0.5) * b.variation;
    float speck = (hash21(uv * b.courses * 64.0) - 0.5) * b.grain * 0.25;
    vec3 face = clamp(b.brick_colour * (1.0 + tint + speck), 0.0, 1.0);
    vec3 colour = mix(b.mortar_colour, face, mask);

    // the normal comes off the same height function, one texel each way
    float step_size = max(u_mat.texel, 1e-5);
    float here = face_height(uv, b);
    float right = face_height(fract(uv + vec2(step_size, 0.0)), b);
    float up = face_height(fract(uv + vec2(0.0, step_size)), b);
    vec3 normal = normalize(vec3(-(right - here) * 8.0, -(up - here) * 8.0, step_size * 8.0));

    // mortar sits in shadow, which is all ambient occlusion is here
    float ao = mix(0.55, 1.0, mask);

    o_albedo = vec4(colour, 1.0);
    o_normal = vec4(normal * 0.5 + 0.5, clamp(here, 0.0, 1.0));
    o_surface = vec4(mix(b.mortar_rough, b.brick_rough, mask), b.metallic, ao, 1.0);
}

// -----------------------------------------------------------------------------------------------------------------------
// the default table: what the editor starts from, and what "Save tuning" writes back

Brick default_brick() {
    Brick b;                              // !(tune=u_mat.brick)
    b.brick_colour = vec3(0.55, 0.24, 0.18);
    b.brick_rough = 0.82;
    b.mortar_colour = vec3(0.74, 0.72, 0.68);
    b.mortar_rough = 0.95;
    b.courses = vec2(4.0, 8.0);
    b.mortar_width = 0.045;
    b.bevel = 0.03;
    b.variation = 0.35;
    b.grain = 0.5;
    b.relief = 1.0;
    b.metallic = 0.0;
    return b;
}
