// WebGL2-compatible gradient shader with reduced inter-stage varyings
// Supports max 4 gradient stops instead of 8 to stay within WebGL2's 31 component limit
// Also supports radial gradients (gradient_type == 1)

// Input must match the native shader's vertex attribute layout exactly
// since they share the same Gradient struct and vertex buffer
struct GradientVertexInput {
    @builtin(vertex_index) vertex_index: u32,
    @location(0) @interpolate(flat) colors_1: vec4<u32>,
    @location(1) @interpolate(flat) colors_2: vec4<u32>,
    @location(2) @interpolate(flat) colors_3: vec4<u32>,
    @location(3) @interpolate(flat) colors_4: vec4<u32>,
    @location(4) @interpolate(flat) offsets: vec4<u32>,
    @location(5) direction: vec4<f32>,  // Linear: start/end, Radial: center/radius
    @location(6) gradient_type: u32,
    @location(7) _padding: vec3<u32>,
    @location(8) position_and_scale: vec4<f32>,
    @location(9) border_color: vec4<f32>,
    @location(10) border_radius: vec4<f32>,
    @location(11) border_width: f32,
    @location(12) shadow_inset: u32,
    @location(13) snap: u32,
}

// Reduced output struct for WebGL2 compatibility
// Only pass colors_1 and colors_2 (4 stops max) and first half of offsets
// Pack radial params into direction (they're mutually exclusive):
//   - Linear: direction = start.xy, end.zw
//   - Radial: direction = center.xy, radius.zw
struct GradientVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) colors_1: vec4<u32>,
    @location(1) @interpolate(flat) colors_2: vec4<u32>,
    @location(2) @interpolate(flat) offsets: vec4<f32>,
    @location(3) direction: vec4<f32>,  // Linear: start/end, Radial: center/radius
    @location(4) @interpolate(flat) gradient_type: u32,
    @location(5) position_and_scale: vec4<f32>,
    @location(6) border_color: vec4<f32>,
    @location(7) border_radius: vec4<f32>,
    @location(8) @interpolate(flat) border_width: f32,
}

@vertex
fn gradient_vs_main(input: GradientVertexInput) -> GradientVertexOutput {
    var out: GradientVertexOutput;

    var pos: vec2<f32> = input.position_and_scale.xy * globals.scale;
    var scale: vec2<f32> = input.position_and_scale.zw * globals.scale;

    var pos_snap = vec2<f32>(0.0, 0.0);
    var scale_snap = vec2<f32>(0.0, 0.0);

    if bool(input.snap) {
        pos_snap = round(pos + vec2(0.001, 0.001)) - pos;
        scale_snap = round(pos + scale + vec2(0.001, 0.001)) - pos - pos_snap - scale;
    }

    var min_border_radius = min(input.position_and_scale.z, input.position_and_scale.w) * 0.5;
    var border_radius: vec4<f32> = vec4<f32>(
        min(input.border_radius.x, min_border_radius),
        min(input.border_radius.y, min_border_radius),
        min(input.border_radius.z, min_border_radius),
        min(input.border_radius.w, min_border_radius)
    );

    var transform: mat4x4<f32> = mat4x4<f32>(
        vec4<f32>(scale.x + scale_snap.x + 1.0, 0.0, 0.0, 0.0),
        vec4<f32>(0.0, scale.y + scale_snap.y + 1.0, 0.0, 0.0),
        vec4<f32>(0.0, 0.0, 1.0, 0.0),
        vec4<f32>(pos + pos_snap - vec2<f32>(0.5, 0.5), 0.0, 1.0)
    );

    out.position = globals.transform * transform * vec4<f32>(vertex_position(input.vertex_index), 0.0, 1.0);
    out.colors_1 = input.colors_1;
    out.colors_2 = input.colors_2;
    
    // Unpack first 4 offsets to f32 in vertex shader
    let offsets_packed: vec4<f32> = unpack_u32(input.offsets.xy);
    out.offsets = offsets_packed;
    
    out.gradient_type = input.gradient_type;
    
    // direction contains: Linear: start.xy/end.zw, Radial: center.xy/radius.zw
    // Just scale it uniformly
    out.direction = input.direction * globals.scale;
    
    out.position_and_scale = vec4<f32>(pos + pos_snap, scale + scale_snap);
    out.border_color = premultiply(input.border_color);
    out.border_radius = border_radius * globals.scale;
    out.border_width = input.border_width * globals.scale;

    return out;
}

fn random(coords: vec2<f32>) -> f32 {
    return fract(sin(dot(coords, vec2(12.9898,78.233))) * 43758.5453);
}

/// Returns the current interpolated color with a max 4-stop linear gradient (WebGL2 compatible)
fn gradient_linear(
    raw_position: vec2<f32>,
    direction: vec4<f32>,
    colors: array<vec4<f32>, 4>,
    offsets: array<f32, 4>,
    last_index: i32
) -> vec4<f32> {
    let start = direction.xy;
    let end = direction.zw;

    let v1 = end - start;
    let v2 = raw_position - start;
    let unit = normalize(v1);
    let coord_offset = dot(unit, v2) / length(v1);

    var colors_arr = colors;
    var offsets_arr = offsets;

    var color: vec4<f32>;

    let noise_granularity: f32 = 0.3/255.0;

    for (var i: i32 = 0; i < last_index; i++) {
        let curr_offset = offsets_arr[i];
        let next_offset = offsets_arr[i+1];

        if (coord_offset <= offsets_arr[0]) {
            color = colors_arr[0];
        }

        if (curr_offset <= coord_offset && coord_offset <= next_offset) {
            let from_ = colors_arr[i];
            let to_ = colors_arr[i+1];
            let factor = smoothstep(curr_offset, next_offset, coord_offset);

            color = interpolate_color(from_, to_, factor);
        }

        if (coord_offset >= offsets_arr[last_index]) {
            color = colors_arr[last_index];
        }
    }

    return color + mix(-noise_granularity, noise_granularity, random(raw_position));
}

/// Returns the current interpolated color with a max 4-stop radial gradient (WebGL2 compatible)
fn gradient_radial(
    raw_position: vec2<f32>,
    center: vec2<f32>,
    radius: vec2<f32>,
    colors: array<vec4<f32>, 4>,
    offsets: array<f32, 4>,
    last_index: i32
) -> vec4<f32> {
    let diff = raw_position - center;
    let normalized_dist = length(diff / radius);

    var colors_arr = colors;
    var offsets_arr = offsets;

    var color: vec4<f32>;

    let noise_granularity: f32 = 0.3/255.0;

    for (var i: i32 = 0; i < last_index; i++) {
        let curr_offset = offsets_arr[i];
        let next_offset = offsets_arr[i+1];

        if (normalized_dist <= offsets_arr[0]) {
            color = colors_arr[0];
        }

        if (curr_offset <= normalized_dist && normalized_dist <= next_offset) {
            let from_ = colors_arr[i];
            let to_ = colors_arr[i+1];
            let factor = smoothstep(curr_offset, next_offset, normalized_dist);

            color = interpolate_color(from_, to_, factor);
        }

        if (normalized_dist >= offsets_arr[last_index]) {
            color = colors_arr[last_index];
        }
    }

    return color + mix(-noise_granularity, noise_granularity, random(raw_position));
}

@fragment
fn gradient_fs_main(input: GradientVertexOutput) -> @location(0) vec4<f32> {
    let colors = array<vec4<f32>, 4>(
        unpack_color(input.colors_1.xy),
        unpack_color(input.colors_1.zw),
        unpack_color(input.colors_2.xy),
        unpack_color(input.colors_2.zw),
    );

    var offsets = array<f32, 4>(
        input.offsets.x,
        input.offsets.y,
        input.offsets.z,
        input.offsets.w,
    );

    // Find last valid index (max 4 stops)
    var last_index = 3;
    for (var i: i32 = 0; i <= 3; i++) {
        if (offsets[i] > 1.0) {
            last_index = i - 1;
            break;
        }
    }

    var mixed_color: vec4<f32>;

    // Check gradient type: 0 = linear, 1 = radial
    // direction contains: Linear: start.xy/end.zw, Radial: center.xy/radius.zw
    if (input.gradient_type == 1u) {
        mixed_color = gradient_radial(
            input.position.xy,
            input.direction.xy,  // center (packed)
            input.direction.zw,  // radius (packed)
            colors,
            offsets,
            last_index
        );
    } else {
        mixed_color = gradient_linear(input.position.xy, input.direction, colors, offsets, last_index);
    }

    let pos = input.position_and_scale.xy;
    let scale = input.position_and_scale.zw;

    var dist: f32 = rounded_box_sdf(
        -(input.position.xy - pos - scale / 2.0) * 2.0,
        scale,
        input.border_radius * 2.0
    ) / 2.0;

    if (input.border_width > 0.0) {
        mixed_color = mix(
            mixed_color,
            input.border_color,
            clamp(0.5 + dist + input.border_width, 0.0, 1.0)
        );
    }

    return mixed_color * clamp(0.5-dist, 0.0, 1.0);
}
