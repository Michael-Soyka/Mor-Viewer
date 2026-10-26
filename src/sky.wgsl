// The game's sky: a cube of five faces around the eye, two sets of them blended by the time of day.

// As `Frame` in model.wgsl; only some fields are read here.
struct Frame {
    view_proj: mat4x4<f32>,
    light: vec4<f32>,
    params: vec4<f32>,
    eye: vec4<f32>,
    ambient: vec4<f32>,
    fog_color: vec4<f32>,
    fog_range: vec4<f32>,
    clip: vec4<f32>,
    water: vec4<f32>,
    sky: vec4<f32>,
    sun: vec4<f32>,
    // x, y: first layer of the two sky sets, z: share of the second, w: 1 when the sky is drawn.
    sky_layers: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
// Faces bk, ft, lt, rt, up of every set; the side faces' lower half continues their horizon.
@group(1) @binding(0) var faces: texture_2d_array<f32>;
@group(1) @binding(1) var faces_sampler: sampler;

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) direction: vec3<f32>,
};

// Corners of the 12 triangles of a cube, seen from inside.
fn corner(index: u32) -> vec3<f32> {
    var sides = array<vec3<u32>, 6>(
        vec3<u32>(0u, 1u, 2u), vec3<u32>(1u, 0u, 2u), vec3<u32>(1u, 2u, 0u),
        vec3<u32>(2u, 1u, 0u), vec3<u32>(2u, 0u, 1u), vec3<u32>(0u, 2u, 1u),
    );
    let quad = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let face = index / 6u;
    let point = quad[index % 6u];
    let axes = sides[face];
    let side = select(-1.0, 1.0, face % 2u == 0u);
    var position = vec3<f32>(0.0);
    position[axes.x] = point.x;
    position[axes.y] = point.y;
    position[axes.z] = side;
    return position;
}

// Drawn first, at the far depth: whatever the frame draws covers it.
// Only the direction is transformed, w = 0: the eye maps to the view's origin, and adding its far-off position
// to the cube's would lose the cube to rounding, the sky shaking as the eye moves.
@vertex
fn vs_sky(@builtin(vertex_index) index: u32) -> VertexOutput {
    let direction = corner(index);
    var output: VertexOutput;
    output.clip = frame.view_proj * vec4<f32>(direction, 0.0);
    output.clip.z = 0.0;
    output.direction = direction;
    return output;
}

// Face layer and its uv for a direction, the game's left-handed world with y up: ft looks along +z, rt along +x.
fn face_uv(direction: vec3<f32>) -> vec3<f32> {
    let d = direction;
    let a = abs(d);
    if (d.y >= a.x && d.y >= a.z) {
        // The top's lower edge meets the front face.
        return vec3<f32>(0.5 + (0.5 * (d.x / d.y)), 0.5 + (0.5 * (d.z / d.y)), 4.0);
    }
    let v = 0.5 - (0.5 * (d.y / max(a.x, a.z)));
    if (a.z >= a.x) {
        let u = 0.5 + (0.5 * (d.x / d.z));
        return vec3<f32>(u, v, select(0.0, 1.0, d.z > 0.0));
    }
    let u = 0.5 - (0.5 * (d.z / d.x));
    return vec3<f32>(u, v, select(2.0, 3.0, d.x > 0.0));
}

// The game's sky sinks into its fog: wholly at the horizon and below, and a haze left overhead. Picked by eye.
const HAZE: f32 = 0.35;
const FADE_HEIGHT: f32 = 0.6;

@fragment
fn fs_sky(input: VertexOutput) -> @location(0) vec4<f32> {
    let direction = normalize(input.direction);
    let face = face_uv(direction);
    let earlier = textureSampleLevel(faces, faces_sampler, face.xy, i32(frame.sky_layers.x + face.z), 0.0).rgb;
    let later = textureSampleLevel(faces, faces_sampler, face.xy, i32(frame.sky_layers.y + face.z), 0.0).rgb;
    let fade = max(HAZE, 1.0 - smoothstep(0.0, FADE_HEIGHT, direction.y)) * frame.fog_color.w;
    return vec4<f32>(mix(mix(earlier, later, frame.sky_layers.z), frame.fog_color.rgb, fade), 1.0);
}
