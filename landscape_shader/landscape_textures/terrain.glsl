#version 450   // !(dialect=unified)
// terrain.glsl: bakes a tileable ground texture from seven terrain types, laid down in patches.
//
// Written to the unified GLSL convention (utils/shader_editor/README.md): one file, both
// stages behind #ifdef guards, uniform blocks with explicit bindings, and a G-buffer of
// fragment outputs that say what each of their lanes means. GLSL 450 as wgpu's naga front end
// reads it (ShaderSource::Glsl with the stage and the defines). Nothing here is real time:
// every type is evaluated at every pixel and the normal costs three evaluations, so bake once
// and reuse the result.
//
// Inputs, all in uniform blocks on group 0. Each holds exactly one struct, and each struct has
// a compiled-in default table at the bottom of this file:
//   binding 0  Globals      Tile         the corner weights, the patch layout, the bake
//   binding 1  StyleBlock   StyleParams  realism vs cartoon: pairs of endpoints and one slider
//   binding 2  TerrainBlock Terrain[7]   one record per type, in the order below
// Compile with USE_DEFAULTS and none of the three bindings is read: the shader bakes from its
// own tables, which is how it is checked and previewed without a host. Otherwise the host
// fills all three, and the default tables are the values it should start from.
//
// Types, by index:
//   0 grass   1 rocks   2 gravel   3 snow   4 sand   5 dirt   6 underwater sand
//
// Weights: tile.weight[t] holds type t's amount at the four tile corners, xyzw = (u0,v0)
// (u1,v0) (u0,v1) (u1,v1), interpolated bilinearly across the tile. An amount is the share
// of the tile the type may cover, not a mix ratio: grass 1.0 and snow 0.5 is grass with
// snow patches over half of it. Where types overlap the one with the higher cover_order
// lies on top, and the border between them is settled by each type's own bumps, so snow
// reaches further over hollows than over stones.
//
// Tiling: all noise is periodic in tile units, so a tile joins itself and any neighbour
// that shares its edge weights. That needs whole-number frequencies; the shader rounds.
// The editor's tiled view (2x2, 4x4) is how you check that it really does join.
//
// Outputs:
//   location 0  albedo    rgb colour, a = 1
//   location 1  normal    xyz tangent-space normal packed 0..1, w = height 0..1
//   location 2  surface   r roughness, g wetness, b winning type / 7, a = 1
// A pixel no type covers comes out magenta so the mistake is visible in the bake.
//
// Row order: under wgpu, clip space has y up while a texture's first row is its top, so the
// vertex stage below flips y. v = 0 is therefore the first row of every target, which is how
// wgpu samples a texture and how the editor reads the bake back. What the preview shows is
// what the PNG holds.

// -----------------------------------------------------------------------------------------------------------------------
// uniforms

const int TYPE_COUNT = 7;
const float TAU = 6.28318530718;

// Everything the tile itself decides: what is where, how big the patches are, how it is baked.
struct Tile {
    // per type: the amount at each corner. A whole tile of one type is 1.0 in all four lanes.
    vec4 weight[TYPE_COUNT];  // !(lanes=[u0v0, u1v0, u0v1, u1v1], names=[grass, rocks, gravel, snow, sand, dirt, sea floor], min=0, max=1)
    float patch_frequency;    // !(min=1, max=16, step=1) patches across one tile edge for the biggest blobs
    float patch_octaves;      // !(min=1, max=6, step=1) extra noise levels breaking the blobs up
    float patch_gain;         // !(min=0.1, max=0.9) how much each extra level contributes, about 0.5
    float patch_warp;         // !(min=0, max=1) curls the patch borders, 0 straight .. 1 very wiggly
    float detail_frequency;   // !(min=2, max=64, step=1) fine detail cells per tile edge, before per-type scaling
    float height_blend;       // !(min=0, max=1) how much a type's bumps push its border out
    float pixel_step;         // !(format=texel) 1 / bake size, the finite difference step for the normal
    float normal_strength;    // !(min=0, max=8) slope exaggeration for the normal map
    float seed;               // !(min=0, max=999, step=1) reroll everything
};

layout(std140, binding = 0) uniform Globals {
    Tile tile;
} u_tile;

// Every realism knob is a pair: the value at realism 0 (cartoon) and at realism 1 (real).
struct StyleParams {
    float realism;            // !(min=0, max=1) the master slider, 0 cartoon .. 1 real
    float bands_cartoon;      // !(min=0, max=16, step=1) colour steps per channel, 0 = off
    float bands_real;         // !(min=0, max=16, step=1)
    float edge_soft_cartoon;  // !(min=0, max=0.5) width of a patch border in noise units
    float edge_soft_real;     // !(min=0, max=0.5)
    float detail_cartoon;     // !(min=0, max=2) bump and shading amplitude multiplier
    float detail_real;        // !(min=0, max=2)
    float saturation_cartoon; // !(min=0, max=2) 1 as authored, above 1 punchier
    float saturation_real;    // !(min=0, max=2)
    float variation_cartoon;  // !(min=0, max=2) colour_a to colour_b variation multiplier
    float variation_real;     // !(min=0, max=2)
    float outline_cartoon;    // !(min=0, max=1) dark line along patch borders, 0 none
    float outline_real;       // !(min=0, max=1)
    float speckle_cartoon;    // !(min=0, max=2) speckle multiplier
    float speckle_real;       // !(min=0, max=2)
};

layout(std140, binding = 1) uniform StyleBlock {
    StyleParams style;
} u_style;

// One terrain type, a named field at a time. The vec3 + float pairs are deliberate: std140
// gives a vec3 sixteen bytes of alignment but only twelve of size, so the float behind one
// rides in the padding and the record stays ninety-six bytes.
struct Terrain {
    vec3  colour_a;      // !(format=RGB) base colour
    float roughness;     // !(min=0, max=1) 0 shiny .. 1 matte
    vec3  colour_b;      // !(format=RGB) second colour, mixed in by the soft noise
    float wetness;       // !(min=0, max=1) 1 tells the water pass it is submerged
    float cover_order;   // !(min=0, max=7, step=1) higher lies on top
    float detail_mul;    // !(min=0.25, max=16) detail frequency multiplier
    float bump;          // !(min=0, max=1.5) bump amplitude
    float cellular;      // !(min=0, max=1) 0 soft noise .. 1 worley cells
    float variation;     // !(min=0, max=2) colour variation between the two colours
    float speckle;       // !(min=0, max=1) speckle amount: blades, sparkle, grit
    float speckle_mul;   // !(min=0.5, max=16) speckle frequency multiplier
    float slabs;         // !(min=0, max=1) cell softness: 0 round domes .. 1 flat slabs
    float patch_mul;     // !(min=0.25, max=4) patch size multiplier
    float border_mul;    // !(min=0.1, max=4) border softness multiplier
    float seed_offset;   // !(min=0, max=32, step=1) so two types do not share a pattern
    float ripple;        // !(min=0, max=1) sand dunes, ripples on the sea floor
    float crack;         // !(min=-1, max=1) + dark sunken cracks, - bright ridges
    float crack_width;   // !(min=0, max=0.5)
    float clumps;        // !(min=0, max=1) extra noise levels on the soft body
    float speckle_tint;  // !(min=-1, max=1) -1 dark speckle .. +1 light speckle
};

layout(std140, binding = 2) uniform TerrainBlock {
    Terrain terrain[TYPE_COUNT];  // !(names=[grass, rocks, gravel, snow, sand, dirt, sea floor])
} u_terrain;

// -----------------------------------------------------------------------------------------------------------------------
// vertex stage: one triangle over the clip square, uv 0..1 across the visible part

#ifdef VERTEX   // !(stage=vertex, verts=3)

layout(location = 0) out vec2 v_uv;

void main() {
    vec2 corner = vec2(float((gl_VertexIndex << 1) & 2), float(gl_VertexIndex & 2));
    v_uv = corner;
    // y is flipped so v = 0 lands on the target's first row, as wgpu samples a texture
    gl_Position = vec4(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
}

#endif

// -----------------------------------------------------------------------------------------------------------------------
// fragment stage

#ifdef FRAGMENT   // !(stage=fragment)

layout(location = 0) in vec2 v_uv;

layout(location = 0) out vec4 o_albedo;    // !(pbr=albedo:rgb) rgb colour, a = 1
layout(location = 1) out vec4 o_normal;    // !(pbr=normal:xyz, pbr=height:w) normal packed 0..1, height in w
layout(location = 2) out vec4 o_surface;   // !(pbr=roughness:r, pbr=wetness:g, pbr=data:b) b is the winning type / 7

// Forward declarations for the default tables at the bottom of the file.
Tile default_tile();
StyleParams default_style();
Terrain default_terrain(int t);

Tile current_tile() {
#ifdef USE_DEFAULTS   // !(toggle="bake from the compiled-in tables, no host data", default=off)
    return default_tile();
#else
    return u_tile.tile;
#endif
}

StyleParams current_style() {
#ifdef USE_DEFAULTS
    return default_style();
#else
    return u_style.style;
#endif
}

Terrain current_terrain(int t) {
#ifdef USE_DEFAULTS
    return default_terrain(t);
#else
    return u_terrain.terrain[t];
#endif
}

// A frequency must be a whole number of cells per tile or the tile stops joining itself.
int whole_frequency(float f) {
    return max(1, int(floor(f + 0.5)));
}

// -----------------------------------------------------------------------------------------------------------------------
// periodic noise

uint hash_u32(uint x) {
    x ^= x >> 16u;
    x *= 0x7feb352du;
    x ^= x >> 15u;
    x *= 0x846ca68bu;
    x ^= x >> 16u;
    return x;
}

// A lattice point wrapped to the period, so everything built on it repeats every period cells.
float hash_lattice(ivec2 p, int period, uint seed) {
    ivec2 span = ivec2(period);
    ivec2 w = ((p % span) + span) % span;
    uint h = hash_u32(uint(w.x) + hash_u32(uint(w.y) + hash_u32(seed)));
    return float(h) * (1.0 / 4294967295.0);
}

vec2 hash_lattice2(ivec2 p, int period, uint seed) {
    return vec2(hash_lattice(p, period, seed), hash_lattice(p, period, seed + 7919u));
}

// Value noise in 0..1 over a lattice of period cells, quintic smoothed.
float value_noise(vec2 p, int period, uint seed) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    ivec2 c = ivec2(i);
    float a = hash_lattice(c, period, seed);
    float b = hash_lattice(c + ivec2(1, 0), period, seed);
    float d = hash_lattice(c + ivec2(0, 1), period, seed);
    float e = hash_lattice(c + ivec2(1, 1), period, seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}

// Fractal value noise in 0..1; each level doubles the frequency so every level stays periodic.
float fbm(vec2 uv, int frequency, int octaves, float gain, uint seed) {
    float sum = 0.0;
    float amp = 1.0;
    float norm = 0.0;
    int period = frequency;
    vec2 p = uv * float(frequency);
    for (int i = 0; i < octaves; i++) {
        sum += amp * value_noise(p, period, seed + uint(i) * 131u);
        norm += amp;
        amp *= gain;
        p *= 2.0;
        period *= 2;
    }
    return sum / max(norm, 1e-6);
}

// Worley cells: distance to the nearest feature point, to the second nearest, and the nearest cell's id.
struct Cells {
    float f1;
    float f2;
    float id;
};

Cells worley(vec2 uv, int frequency, uint seed) {
    vec2 p = uv * float(frequency);
    ivec2 base = ivec2(floor(p));
    vec2 f = fract(p);
    Cells c;
    c.f1 = 8.0;
    c.f2 = 8.0;
    c.id = 0.0;
    for (int y = -1; y <= 1; y++) {
        for (int x = -1; x <= 1; x++) {
            ivec2 n = ivec2(x, y);
            vec2 feature = vec2(n) + hash_lattice2(base + n, frequency, seed);
            float d = length(feature - f);
            if (d < c.f1) {
                c.f2 = c.f1;
                c.f1 = d;
                c.id = hash_lattice(base + n, frequency, seed + 17u);
            } else if (d < c.f2) {
                c.f2 = d;
            }
        }
    }
    return c;
}

// -----------------------------------------------------------------------------------------------------------------------
// one terrain type on its own

struct Surface {
    vec3 colour;
    float height;
    float roughness;
    float wetness;
};

// What type t looks like at uv if it covered the whole tile.
Surface evaluate_type(int t, vec2 uv, Terrain k, Tile tile, StyleParams s, float realism) {
    float style_detail = mix(s.detail_cartoon, s.detail_real, realism);
    float style_variation = mix(s.variation_cartoon, s.variation_real, realism);
    float style_speckle = mix(s.speckle_cartoon, s.speckle_real, realism);

    int frequency = whole_frequency(tile.detail_frequency * k.detail_mul);
    uint seed = uint(tile.seed) * 65537u + uint(k.seed_offset) * 1009u + uint(t) * 7777u;

    // The soft body: fractal noise, the clumps knob adding finer levels on top.
    int octaves = 2 + int(floor(k.clumps * 3.0 + 0.5));
    float soft = fbm(uv, frequency, octaves, 0.5, seed);

    // The cellular body: domes for pebbles, flat slabs of differing height for rock.
    Cells cells = worley(uv, frequency, seed + 3u);
    float dome = clamp(1.0 - cells.f1 * 1.2, 0.0, 1.0);
    float slab = mix(0.35, 1.0, cells.id);
    float cellular = mix(dome, slab, k.slabs);

    float height = mix(soft, cellular, k.cellular);

    // Ripples: a sine across the tile, bent by the soft noise; frequency doubled so it tiles.
    float ripple = 0.5 + 0.5 * sin(TAU * (uv.x * float(frequency) * 2.0 + soft * 1.5));
    height += k.ripple * (ripple - 0.5) * 0.5;

    // Cracks along the cell borders: dark and sunken, or with a negative strength bright ridges.
    float crack_mask = 1.0 - smoothstep(0.0, max(k.crack_width, 1e-4), cells.f2 - cells.f1);
    height -= crack_mask * k.crack * 0.5;

    // Colour: the two authored colours mixed by the soft noise, then lit a little by height.
    float variation = clamp(0.5 + (soft - 0.5) * k.variation * style_variation * 2.0, 0.0, 1.0);
    vec3 colour = mix(k.colour_a, k.colour_b, variation);
    colour *= 1.0 - crack_mask * k.crack * 0.6;
    colour *= mix(1.0, mix(0.8, 1.1, height), style_detail);

    // Speckle: grass blades, sparkle on snow, grit in the dirt.
    int speckle_frequency = whole_frequency(float(frequency) * k.speckle_mul);
    float grain = value_noise(uv * float(speckle_frequency), speckle_frequency, seed + 11u);
    float speckle = smoothstep(0.62, 0.9, grain) * k.speckle * style_speckle;
    colour = mix(colour, colour * (1.0 + k.speckle_tint * 0.6), speckle);
    height += speckle * 0.15;

    Surface out_surface;
    out_surface.colour = clamp(colour, 0.0, 1.0);
    out_surface.height = clamp(height, 0.0, 1.0) * k.bump * style_detail;
    out_surface.roughness = k.roughness;
    out_surface.wetness = k.wetness;
    return out_surface;
}

// -----------------------------------------------------------------------------------------------------------------------
// the patchy blend

// How much of type t claims the pixel at uv: 1 inside a patch, 0 outside, soft at the border.
float presence(int t, vec2 uv, Terrain k, Tile tile, float amount, float edge_soft) {
    if (amount <= 0.0) {
        return 0.0;
    }
    if (amount >= 1.0) {
        return 1.0;
    }
    int frequency = whole_frequency(tile.patch_frequency * k.patch_mul);
    uint seed = uint(tile.seed) * 65537u + uint(k.seed_offset) * 1009u + uint(t) * 4099u + 500u;
    int octaves = clamp(1 + int(floor(tile.patch_octaves + 0.5)), 1, 8);

    // Domain warp bends the borders so patches stop looking like blurred blobs.
    vec2 warp = vec2(fbm(uv, frequency, 2, 0.5, seed + 1u), fbm(uv, frequency, 2, 0.5, seed + 2u)) - 0.5;
    vec2 p = uv + warp * tile.patch_warp / float(frequency);

    // A patch is where the noise falls below the amount, so an amount of 0.5 covers about half.
    float n = fbm(p, frequency, octaves, tile.patch_gain, seed);
    float soft = max(edge_soft * k.border_mul, 1e-4);
    return smoothstep(-soft, soft, amount - n);
}

struct Blend {
    Surface surface;
    int winner;
    float edge;
};

// Bilinear weights of the four corners at uv, in the (u0,v0) (u1,v0) (u0,v1) (u1,v1) order.
vec4 corner_basis(vec2 uv) {
    return vec4((1.0 - uv.x) * (1.0 - uv.y), uv.x * (1.0 - uv.y), (1.0 - uv.x) * uv.y, uv.x * uv.y);
}

// The whole tile at uv: every type laid down in cover order, each over the ones below it.
Blend evaluate(vec2 uv, Tile tile, StyleParams s) {
    float realism = clamp(s.realism, 0.0, 1.0);
    float edge_soft = mix(s.edge_soft_cartoon, s.edge_soft_real, realism);
    vec4 basis = corner_basis(uv);

    Surface surfaces[TYPE_COUNT];
    float coverage[TYPE_COUNT];
    float order[TYPE_COUNT];
    for (int t = 0; t < TYPE_COUNT; t++) {
        Terrain k = current_terrain(t);
        float amount = clamp(dot(tile.weight[t], basis), 0.0, 1.0);
        coverage[t] = presence(t, uv, k, tile, amount, edge_soft);
        surfaces[t] = evaluate_type(t, uv, k, tile, s, realism);
        order[t] = k.cover_order;
    }

    // Nothing covering the pixel leaves this magenta: a bake with holes is a bake to look at.
    Blend result;
    result.surface.colour = vec3(1.0, 0.0, 1.0);
    result.surface.height = 0.5;
    result.surface.roughness = 1.0;
    result.surface.wetness = 0.0;
    result.winner = -1;
    result.edge = 0.0;

    // Composite bottom to top: each pass takes the lowest type not yet laid down.
    bool done[TYPE_COUNT];
    for (int t = 0; t < TYPE_COUNT; t++) {
        done[t] = false;
    }
    for (int pass = 0; pass < TYPE_COUNT; pass++) {
        int pick = -1;
        for (int t = 0; t < TYPE_COUNT; t++) {
            if (!done[t] && (pick < 0 || order[t] < order[pick])) {
                pick = t;
            }
        }
        done[pick] = true;
        float a = coverage[pick];
        if (a <= 0.0) {
            continue;
        }
        // Bumps settle the border: only the partial band moves, by the height difference.
        float band = a * (1.0 - a) * 4.0;
        float lift = (surfaces[pick].height - result.surface.height) * tile.height_blend;
        a = clamp(a + lift * band, 0.0, 1.0);
        band = a * (1.0 - a) * 4.0;

        result.surface.colour = mix(result.surface.colour, surfaces[pick].colour, a);
        result.surface.height = mix(result.surface.height, surfaces[pick].height, a);
        result.surface.roughness = mix(result.surface.roughness, surfaces[pick].roughness, a);
        result.surface.wetness = mix(result.surface.wetness, surfaces[pick].wetness, a);
        result.edge = max(result.edge * (1.0 - a), band);
        if (a >= 0.5) {
            result.winner = pick;
        }
    }
    return result;
}

// -----------------------------------------------------------------------------------------------------------------------
// style and output

vec3 stylise(vec3 colour, float edge, StyleParams s, float realism) {
    float bands = mix(s.bands_cartoon, s.bands_real, realism);
    float saturation = mix(s.saturation_cartoon, s.saturation_real, realism);
    float outline = mix(s.outline_cartoon, s.outline_real, realism);

    float luma = dot(colour, vec3(0.299, 0.587, 0.114));
    colour = mix(vec3(luma), colour, saturation);
    if (bands >= 1.0) {
        colour = floor(colour * bands + 0.5) / bands;
    }
    colour *= 1.0 - edge * outline;
    return clamp(colour, 0.0, 1.0);
}

void main() {
    Tile tile = current_tile();
    StyleParams s = current_style();
    float realism = clamp(s.realism, 0.0, 1.0);

    Blend here = evaluate(v_uv, tile, s);
    // The neighbours wrap because the tile is periodic, so the normal joins across the seam too.
    float step_size = max(tile.pixel_step, 1e-5);
    float right = evaluate(fract(v_uv + vec2(step_size, 0.0)), tile, s).surface.height;
    float up = evaluate(fract(v_uv + vec2(0.0, step_size)), tile, s).surface.height;
    float height = here.surface.height;
    vec3 normal = normalize(vec3(-(right - height) * tile.normal_strength, -(up - height) * tile.normal_strength, step_size));

    o_albedo = vec4(stylise(here.surface.colour, here.edge, s, realism), 1.0);
    o_normal = vec4(normal * 0.5 + 0.5, clamp(height, 0.0, 1.0));
    float winner = here.winner < 0 ? 1.0 : float(here.winner) / float(TYPE_COUNT);
    o_surface = vec4(here.surface.roughness, here.surface.wetness, winner, 1.0);
}

// -----------------------------------------------------------------------------------------------------------------------
// defaults: the values the host's uniform buffers should start from, and what USE_DEFAULTS bakes
// with. The !(tune=) markers make these the editor's starting values too, and "Save tuning"
// writes the sliders back onto these very lines.

Tile default_tile() {
    Tile t;                       // !(tune=u_tile.tile)
    // a grassy tile with dirt showing through, rocks along one edge and a little gravel
    t.weight[0] = vec4(0.85, 0.9, 0.7, 0.95);
    t.weight[1] = vec4(0.0, 0.5, 0.0, 0.25);
    t.weight[2] = vec4(0.2, 0.2, 0.0, 0.0);
    t.weight[3] = vec4(0.0, 0.0, 0.0, 0.0);
    t.weight[4] = vec4(0.0, 0.0, 0.0, 0.0);
    t.weight[5] = vec4(0.35, 0.0, 0.6, 0.2);
    t.weight[6] = vec4(0.0, 0.0, 0.0, 0.0);
    t.patch_frequency = 3.0;
    t.patch_octaves = 3.0;
    t.patch_gain = 0.5;
    t.patch_warp = 0.6;
    t.detail_frequency = 16.0;
    t.height_blend = 0.5;
    t.pixel_step = 0.0009765625;  // 1/1024, the bake size this was authored at
    t.normal_strength = 2.0;
    t.seed = 0.0;
    return t;
}

StyleParams default_style() {
    StyleParams s;            // !(tune=u_style.style)
    s.realism = 0.7;
    s.bands_cartoon = 5.0;
    s.bands_real = 0.0;
    s.edge_soft_cartoon = 0.03;
    s.edge_soft_real = 0.18;
    s.detail_cartoon = 0.5;
    s.detail_real = 1.0;
    s.saturation_cartoon = 1.35;
    s.saturation_real = 1.0;
    s.variation_cartoon = 0.4;
    s.variation_real = 1.0;
    s.outline_cartoon = 0.5;
    s.outline_real = 0.0;
    s.speckle_cartoon = 0.3;
    s.speckle_real = 1.0;
    return s;
}

// Fields as documented on the Terrain struct above.
Terrain default_terrain(int t) {
    Terrain k;
    switch (t) {
        case 0:   // !(tune=u_terrain.terrain[0]) grass: two greens, fine soft noise, dense blade speckle, low on the pile
            k.colour_a = vec3(0.33, 0.52, 0.18);
            k.roughness = 0.85;
            k.colour_b = vec3(0.5, 0.62, 0.22);
            k.wetness = 0.0;
            k.cover_order = 2.0;
            k.detail_mul = 6.0;
            k.bump = 0.35;
            k.cellular = 0.0;
            k.variation = 0.8;
            k.speckle = 0.6;
            k.speckle_mul = 4.0;
            k.slabs = 0.0;
            k.patch_mul = 1.0;
            k.border_mul = 1.0;
            k.seed_offset = 0.0;
            k.ripple = 0.0;
            k.crack = 0.0;
            k.crack_width = 0.2;
            k.clumps = 0.6;
            k.speckle_tint = 0.5;
            break;
        case 1:   // !(tune=u_terrain.terrain[1]) rocks: large flat slabs with dark cracks, tall, near the top of the pile
            k.colour_a = vec3(0.45, 0.43, 0.4);
            k.roughness = 0.7;
            k.colour_b = vec3(0.3, 0.29, 0.3);
            k.wetness = 0.0;
            k.cover_order = 5.0;
            k.detail_mul = 1.5;
            k.bump = 1.0;
            k.cellular = 0.9;
            k.variation = 0.6;
            k.speckle = 0.15;
            k.speckle_mul = 6.0;
            k.slabs = 0.8;
            k.patch_mul = 0.7;
            k.border_mul = 0.6;
            k.seed_offset = 1.0;
            k.ripple = 0.0;
            k.crack = 0.9;
            k.crack_width = 0.12;
            k.clumps = 0.3;
            k.speckle_tint = -0.5;
            break;
        case 2:   // !(tune=u_terrain.terrain[2]) gravel: small round pebbles in mixed greys
            k.colour_a = vec3(0.55, 0.52, 0.48);
            k.roughness = 0.8;
            k.colour_b = vec3(0.4, 0.38, 0.36);
            k.wetness = 0.0;
            k.cover_order = 4.0;
            k.detail_mul = 8.0;
            k.bump = 0.6;
            k.cellular = 1.0;
            k.variation = 0.9;
            k.speckle = 0.3;
            k.speckle_mul = 4.0;
            k.slabs = 0.1;
            k.patch_mul = 1.2;
            k.border_mul = 1.0;
            k.seed_offset = 2.0;
            k.ripple = 0.0;
            k.crack = 0.4;
            k.crack_width = 0.1;
            k.clumps = 0.2;
            k.speckle_tint = 0.4;
            break;
        case 3:   // !(tune=u_terrain.terrain[3]) snow: smooth, white over a blue shadow, sparkle, covers everything
            k.colour_a = vec3(0.94, 0.96, 1.0);
            k.roughness = 0.35;
            k.colour_b = vec3(0.8, 0.86, 0.96);
            k.wetness = 0.0;
            k.cover_order = 6.0;
            k.detail_mul = 1.0;
            k.bump = 0.25;
            k.cellular = 0.0;
            k.variation = 0.4;
            k.speckle = 0.5;
            k.speckle_mul = 8.0;
            k.slabs = 0.0;
            k.patch_mul = 1.0;
            k.border_mul = 1.4;
            k.seed_offset = 3.0;
            k.ripple = 0.0;
            k.crack = 0.0;
            k.crack_width = 0.0;
            k.clumps = 0.4;
            k.speckle_tint = 1.0;
            break;
        case 4:   // !(tune=u_terrain.terrain[4]) sand: tan, fine, with wind ripples
            k.colour_a = vec3(0.82, 0.72, 0.5);
            k.roughness = 0.75;
            k.colour_b = vec3(0.72, 0.62, 0.42);
            k.wetness = 0.0;
            k.cover_order = 3.0;
            k.detail_mul = 3.0;
            k.bump = 0.3;
            k.cellular = 0.0;
            k.variation = 0.5;
            k.speckle = 0.3;
            k.speckle_mul = 6.0;
            k.slabs = 0.0;
            k.patch_mul = 1.0;
            k.border_mul = 1.2;
            k.seed_offset = 4.0;
            k.ripple = 0.6;
            k.crack = 0.0;
            k.crack_width = 0.0;
            k.clumps = 0.3;
            k.speckle_tint = 0.6;
            break;
        case 5:   // !(tune=u_terrain.terrain[5]) dirt: brown, clods half way to cellular, dark grit
            k.colour_a = vec3(0.42, 0.3, 0.18);
            k.roughness = 0.9;
            k.colour_b = vec3(0.3, 0.21, 0.12);
            k.wetness = 0.0;
            k.cover_order = 1.0;
            k.detail_mul = 3.0;
            k.bump = 0.45;
            k.cellular = 0.35;
            k.variation = 0.7;
            k.speckle = 0.35;
            k.speckle_mul = 5.0;
            k.slabs = 0.2;
            k.patch_mul = 1.0;
            k.border_mul = 1.0;
            k.seed_offset = 5.0;
            k.ripple = 0.0;
            k.crack = 0.3;
            k.crack_width = 0.15;
            k.clumps = 0.5;
            k.speckle_tint = -0.6;
            break;
        default:  // !(tune=u_terrain.terrain[6]) underwater sand: pale, smooth, soft cells with bright ridges, wet, the bottom of the pile
            k.colour_a = vec3(0.7, 0.68, 0.52);
            k.roughness = 0.3;
            k.colour_b = vec3(0.6, 0.6, 0.48);
            k.wetness = 1.0;
            k.cover_order = 0.0;
            k.detail_mul = 2.0;
            k.bump = 0.15;
            k.cellular = 0.6;
            k.variation = 0.3;
            k.speckle = 0.05;
            k.speckle_mul = 4.0;
            k.slabs = 0.4;
            k.patch_mul = 1.0;
            k.border_mul = 1.5;
            k.seed_offset = 6.0;
            k.ripple = 0.3;
            k.crack = -0.5;
            k.crack_width = 0.35;
            k.clumps = 0.2;
            k.speckle_tint = 0.5;
            break;
    }
    return k;
}

#endif
