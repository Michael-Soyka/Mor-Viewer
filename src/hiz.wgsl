// Depth pyramid: each texel keeps the farthest depth under it. Depth is reversed, so that is the smallest.

@group(0) @binding(0) var depth: texture_depth_multisampled_2d;
@group(0) @binding(1) var level: texture_storage_2d<r32float, write>;
@group(0) @binding(2) var source: texture_2d<f32>;

@compute @workgroup_size(8, 8)
fn base(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(level);
    if (any(id.xy >= size)) {
        return;
    }
    var far = 1.0;
    for (var sample = 0u; sample < textureNumSamples(depth); sample++) {
        far = min(far, textureLoad(depth, id.xy, i32(sample)));
    }
    textureStore(level, id.xy, vec4<f32>(far));
}

@compute @workgroup_size(8, 8)
fn down(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(level);
    if (any(id.xy >= size)) {
        return;
    }
    // The last row and column also take the texel an odd size leaves over.
    let start = id.xy * 2u;
    let end = select(start + 1u, textureDimensions(source) - 1u, id.xy == size - 1u);

    var far = 1.0;
    for (var y = start.y; y <= end.y; y++) {
        for (var x = start.x; x <= end.x; x++) {
            far = min(far, textureLoad(source, vec2<u32>(x, y), 0).r);
        }
    }
    textureStore(level, id.xy, vec4<f32>(far));
}
