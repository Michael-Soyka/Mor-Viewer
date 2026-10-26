struct Frame {
    view_proj: mat4x4<f32>,
    light: vec4<f32>,
    // x: render mode (0 wireframe, 1 solid, 2 material, 3 shaded), y: lighting on, z: lightmaps on.
    params: vec4<f32>,
    eye: vec4<f32>,
    // rgb: the scene's ambient light, w: 1 when it replaces the sun.
    ambient: vec4<f32>,
    // rgb: fog color, w: 1 when on.
    fog_color: vec4<f32>,
    // x: distance the fog starts at, y: where it hides everything.
    fog_range: vec4<f32>,
    // Kept side of a reflection's mirror plane, dot(xyz, position) + w >= 0; (0, 0, 0, 1) keeps everything.
    clip: vec4<f32>,
    // x: height of the mirrored water level, y: 1 when its reflection is drawn, zw: 1 / frame size in pixels.
    water: vec4<f32>,
    // rgb: the background, what water reflects without a reflection.
    sky: vec4<f32>,
};

// Below the mirror plane of a reflection pass.
fn clipped(position: vec3<f32>) -> bool {
    return (dot(frame.clip.xyz, position) + frame.clip.w) < 0.0;
}

// Share of the fog color at a point: linear in the distance from the eye, as the game's range-based fog.
fn fog_amount(position: vec3<f32>) -> f32 {
    let distance = length(position - frame.eye.xyz);
    let amount = clamp((distance - frame.fog_range.x) / max(frame.fog_range.y - frame.fog_range.x, 1.0), 0.0, 1.0);
    return amount * frame.fog_color.w;
}

struct Part {
    // rgb: base color, a: 1 when alpha-tested.
    color: vec4<f32>,
    // rgb: color of the light a lightmap overlay is baked for.
    light: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var albedo: texture_2d<f32>;
@group(1) @binding(1) var albedo_sampler: sampler;
@group(1) @binding(2) var<uniform> part: Part;
@group(1) @binding(3) var lightmap: texture_2d<f32>;
// The scene mirrored in the water level, at the same screen positions as the frame.
@group(2) @binding(0) var reflection: texture_2d<f32>;
@group(2) @binding(1) var reflection_sampler: sampler;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) lightmap_uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) position: vec3<f32>,
    @location(3) lightmap_uv: vec2<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.clip = frame.view_proj * vec4<f32>(input.position, 1.0);
    output.normal = input.normal;
    output.uv = input.uv;
    output.position = input.position;
    output.lightmap_uv = input.lightmap_uv;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let mode = frame.params.x;
    let texel = textureSample(albedo, albedo_sampler, input.uv);
    if (mode >= 1.5 && part.color.a > 0.5 && texel.a < 0.5) {
        discard;
    }
    // Two-sided lighting: the game draws most surfaces without culling, so the normal faces the viewer.
    var normal = normalize(input.normal);
    if (dot(normal, frame.eye.xyz - input.position) < 0.0) {
        normal = -normal;
    }
    let diffuse = max(dot(normal, frame.light.xyz), 0.0);
    let lighting = mix(1.0, 0.35 + (0.65 * diffuse), frame.params.y);
    if (mode < 1.5) {
        return vec4<f32>(vec3<f32>(0.78) * lighting, 1.0);
    }
    let base = texel.rgb * part.color.rgb;
    let fog = fog_amount(input.position);
    if (mode < 2.5) {
        return vec4<f32>(mix(base, frame.fog_color.rgb, fog), 1.0);
    }
    // Interiors have no sun, only their ambient light; lightmap overlays add the rest.
    let surrounding = select(vec3<f32>(lighting), frame.ambient.rgb, frame.ambient.w > 0.5);
    return vec4<f32>(mix(base * surrounding, frame.fog_color.rgb, fog), 1.0);
}

@fragment
fn fs_wire(input: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(0.85, 0.85, 0.85, 1.0);
}

// One light's baked share, blended additively over the plain pass:
// the game's MULTIPLYADD of lightmap and light color, then MODULATE2X with the texture.
@fragment
fn fs_lightmap(input: VertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(albedo, albedo_sampler, input.uv);
    let baked = textureSample(lightmap, albedo_sampler, input.lightmap_uv).r;
    if ((part.color.a > 0.5 && texel.a < 0.5) || clipped(input.position)) {
        discard;
    }
    // Added light fades with the fog, as the plain pass under it does.
    let fade = 1.0 - fog_amount(input.position);
    return vec4<f32>(texel.rgb * part.color.rgb * part.light.rgb * (2.0 * baked) * fade, 1.0);
}

// Path lines carry their color in the normal slot.
@fragment
fn fs_path(input: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(input.normal, 1.0);
}
