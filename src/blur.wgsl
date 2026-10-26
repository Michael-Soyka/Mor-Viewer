// Backdrop blur: halving copies, then a separable gaussian at the smallest size.

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

// One triangle over the whole target.
@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));

    var out: VertexOutput;
    out.position = vec4<f32>((uv * vec2<f32>(2.0, -2.0)) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    out.uv = uv;
    return out;
}

// Linear filtering at half size averages each 2x2 block.
@fragment
fn fs_down(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(source, source_sampler, in.uv);
}

// Nine taps in five linear fetches.
fn gaussian(uv: vec2<f32>, step: vec2<f32>) -> vec4<f32> {
    var color = textureSample(source, source_sampler, uv) * 0.2270270270;
    color += textureSample(source, source_sampler, uv + (step * 1.3846153846)) * 0.3162162162;
    color += textureSample(source, source_sampler, uv - (step * 1.3846153846)) * 0.3162162162;
    color += textureSample(source, source_sampler, uv + (step * 3.2307692308)) * 0.0702702703;
    color += textureSample(source, source_sampler, uv - (step * 3.2307692308)) * 0.0702702703;
    return color;
}

@fragment
fn fs_horizontal(in: VertexOutput) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(source));
    return gaussian(in.uv, vec2<f32>(texel.x, 0.0));
}

@fragment
fn fs_vertical(in: VertexOutput) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(source));
    return gaussian(in.uv, vec2<f32>(0.0, texel.y));
}
