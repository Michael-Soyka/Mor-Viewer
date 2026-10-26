// Cluster culling: the view frustum, then the depth pyramid of what is already drawn.
// Depth is reversed, 1 at the near plane and 0 far away.

struct Cull {
    view_proj: mat4x4<f32>,
    // Kept side of a mirror plane, dot(xyz, point) + w >= 0; (0, 0, 0, 1) keeps everything.
    plane: vec4<f32>,
    // Texels of the pyramid's base level.
    size: vec2<u32>,
    mips: u32,
    // OCCLUSION: test against the pyramid; TWO_PHASE: an early pass drew last frame's clusters.
    flags: u32,
};

// Index ranges of one part in one cluster, culled with the cluster.
struct Record {
    cluster: u32,
    overlay: u32,
    first_triangle: u32,
    triangle_count: u32,
    first_edge: u32,
    edge_count: u32,
    part: u32,
    // The part's first record: its draws are packed from there.
    base: u32,
};

struct Projected {
    // Entirely past one plane of the frustum, or under the mirror plane.
    outside: bool,
    // Reaches in front of the near plane, so its screen footprint is unbounded.
    near: bool,
    // Screen bounds in normalized device coordinates, and the nearest depth.
    low: vec2<f32>,
    high: vec2<f32>,
    depth: f32,
};

const VISIBLE: u32 = 1u;
const DRAWN: u32 = 2u;

const OCCLUSION: u32 = 1u;
const TWO_PHASE: u32 = 2u;

@group(0) @binding(0) var<uniform> cull: Cull;
// Minimum and maximum corner of each cluster.
@group(0) @binding(1) var<storage, read> clusters: array<vec4<f32>>;
// VISIBLE: passed the last cull; DRAWN: drawn by this frame's early pass.
@group(0) @binding(2) var<storage, read_write> states: array<u32>;
@group(0) @binding(3) var<storage, read> records: array<Record>;
// DrawIndexedIndirectArgs: triangle draws of all records, then edge draws; each part's visible ones packed at its start.
@group(0) @binding(4) var<storage, read_write> indirect: array<u32>;
// Visible clusters, drawn triangles.
@group(0) @binding(5) var<storage, read_write> counters: array<atomic<u32>, 2>;
@group(0) @binding(6) var pyramid: texture_2d<f32>;
// Packed draws of each part: triangles of all parts, then edges.
@group(0) @binding(7) var<storage, read_write> counts: array<atomic<u32>>;

const TRIANGLES: u32 = 0u;
const EDGES: u32 = 1u;

fn project(index: u32) -> Projected {
    let low = clusters[2u * index].xyz;
    let high = clusters[(2u * index) + 1u].xyz;

    var result: Projected;
    result.near = false;
    result.low = vec2<f32>(1.0e9);
    result.high = vec2<f32>(-1.0e9);
    result.depth = 0.0;

    var left = true;
    var right = true;
    var below = true;
    var above = true;
    var before = true;
    var beyond = true;
    var under = true;

    for (var corner = 0u; corner < 8u; corner++) {
        let pick = vec3<bool>((corner & 1u) != 0u, (corner & 2u) != 0u, (corner & 4u) != 0u);
        let point = select(low, high, pick);
        let clip = cull.view_proj * vec4<f32>(point, 1.0);
        under = under && (dot(cull.plane.xyz, point) + cull.plane.w) < 0.0;
        left = left && clip.x < -clip.w;
        right = right && clip.x > clip.w;
        below = below && clip.y < -clip.w;
        above = above && clip.y > clip.w;
        before = before && clip.z > clip.w;
        beyond = beyond && clip.z < 0.0;
        if (clip.z > clip.w || clip.w <= 0.0) {
            result.near = true;
        } else {
            let device = clip.xyz / clip.w;
            result.low = min(result.low, device.xy);
            result.high = max(result.high, device.xy);
            result.depth = max(result.depth, device.z);
        }
    }

    result.outside = left || right || below || above || before || beyond || under;
    return result;
}

// Hidden when its nearest point is farther than everything drawn over its footprint.
fn occluded(box: Projected) -> bool {
    if (box.near) {
        return false;
    }
    let size = vec2<f32>(cull.size);
    let low = clamp((vec2<f32>(box.low.x, -box.high.y) * 0.5) + 0.5, vec2<f32>(0.0), vec2<f32>(1.0)) * size;
    let high = clamp((vec2<f32>(box.high.x, -box.low.y) * 0.5) + 0.5, vec2<f32>(0.0), vec2<f32>(1.0)) * size;

    // The level where the footprint spans at most two texels each way.
    let extent = max(high.x - low.x, high.y - low.y);
    let level = min(u32(ceil(log2(max(extent, 1.0)))), cull.mips - 1u);
    let last = textureDimensions(pyramid, level) - 1u;
    let first_texel = min(vec2<u32>(low) >> vec2<u32>(level), last);
    let last_texel = min(vec2<u32>(high) >> vec2<u32>(level), last);

    var far = 1.0;
    for (var y = first_texel.y; y <= last_texel.y; y++) {
        for (var x = first_texel.x; x <= last_texel.x; x++) {
            far = min(far, textureLoad(pyramid, vec2<u32>(x, y), level).r);
        }
    }
    return box.depth < far;
}

// Appends a draw to the record's part in `list`; the rest of the part's slots stay cleared.
fn emit(list: u32, record: Record, first: u32, count: u32) {
    let parts = arrayLength(&counts) / 2u;
    let slot = atomicAdd(&counts[(list * parts) + record.part], 1u);
    let base = 5u * ((list * arrayLength(&records)) + record.base + slot);
    indirect[base] = count;
    indirect[base + 1u] = 1u;
    indirect[base + 2u] = first;
    indirect[base + 3u] = 0u;
    indirect[base + 4u] = 0u;
}

// Early pass: last frame's visible clusters still in the frustum.
@compute @workgroup_size(64)
fn early_clusters(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = id.x;
    if (index >= arrayLength(&states)) {
        return;
    }
    let state = states[index] & VISIBLE;
    let drawn = state != 0u && !project(index).outside;
    states[index] = state | select(0u, DRAWN, drawn);
}

@compute @workgroup_size(64)
fn early_draws(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = id.x;
    if (index >= arrayLength(&records)) {
        return;
    }
    let record = records[index];
    if (record.overlay == 0u && (states[record.cluster] & DRAWN) != 0u) {
        emit(TRIANGLES, record, record.first_triangle, record.triangle_count);
    }
}

@compute @workgroup_size(64)
fn cull_clusters(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = id.x;
    if (index >= arrayLength(&states)) {
        return;
    }
    let box = project(index);
    var visible = !box.outside;
    if (visible && (cull.flags & OCCLUSION) != 0u) {
        visible = !occluded(box);
    }
    let drawn = select(0u, states[index] & DRAWN, (cull.flags & TWO_PHASE) != 0u);
    states[index] = select(0u, VISIBLE, visible) | drawn;
    if (visible) {
        atomicAdd(&counters[0], 1u);
    }
}

// Late pass: what became visible, every visible overlay and every visible edge.
@compute @workgroup_size(64)
fn late_draws(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = id.x;
    if (index >= arrayLength(&records)) {
        return;
    }
    let record = records[index];
    let state = states[record.cluster];
    if ((state & VISIBLE) == 0u) {
        return;
    }
    let fresh = (state & DRAWN) == 0u;
    if (fresh || record.overlay != 0u) {
        emit(TRIANGLES, record, record.first_triangle, record.triangle_count);
    }
    if (record.edge_count != 0u) {
        emit(EDGES, record, record.first_edge, record.edge_count);
    }
    if (record.overlay == 0u) {
        atomicAdd(&counters[1], record.triangle_count / 3u);
    }
}
