struct SolidVertexInput {
    @builtin(vertex_index) vertex_index: u32,
    @location(0) color: vec4<f32>,
    @location(1) pos: vec2<f32>,
    @location(2) scale: vec2<f32>,
    @location(3) border_color: vec4<f32>,
    @location(4) border_radius: vec4<f32>,
    @location(5) border_widths: vec4<f32>,
    @location(6) shadow_color: vec4<f32>,
    @location(7) shadow_offset: vec2<f32>,
    @location(8) shadow_blur_radius: f32,
    @location(9) shadow_inset: u32,
    @location(10) shadow_spread_radius: f32,
    @location(11) snap: u32,
    @location(12) border_only: u32,
    @location(13) border_dash: vec2<f32>,
}

struct SolidVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) border_color: vec4<f32>,
    @location(2) pos: vec2<f32>,
    @location(3) scale: vec2<f32>,
    @location(4) border_radius: vec4<f32>,
    @location(5) border_widths: vec4<f32>,
    @location(6) shadow_color: vec4<f32>,
    @location(7) shadow_offset: vec2<f32>,
    @location(8) shadow_blur_radius: f32,
    @location(9) @interpolate(flat) shadow_inset: u32,
    @location(10) shadow_spread_radius: f32,
    @location(11) border_dash: vec2<f32>,
}

@vertex
fn solid_vs_main(input: SolidVertexInput) -> SolidVertexOutput {
    var out: SolidVertexOutput;

    let box_pos = input.pos * globals.scale;
    let box_size = input.scale * globals.scale;

    var pos_snap = vec2<f32>(0.0, 0.0);
    var scale_snap = vec2<f32>(0.0, 0.0);

    if bool(input.snap) {
        pos_snap = round(box_pos + vec2(0.001, 0.001)) - box_pos;
        scale_snap = round(box_pos + box_size + vec2(0.001, 0.001)) - box_pos - pos_snap - box_size;
    }

    // An outset shadow reaches past the box by its offset, its spread and three
    // standard deviations (half the blur radius each) of its blur.
    var reach_before = vec2<f32>(0.0, 0.0);
    var reach_after = vec2<f32>(0.0, 0.0);
    if !bool(input.shadow_inset) {
        let reach = 1.5 * input.shadow_blur_radius + max(input.shadow_spread_radius, 0.0);
        reach_before = (max(-input.shadow_offset, vec2(0.0)) + reach) * globals.scale;
        reach_after = (max(input.shadow_offset, vec2(0.0)) + reach) * globals.scale;
    }

    let pos = box_pos + pos_snap - reach_before;
    let scale = box_size + scale_snap + reach_before + reach_after;

    let border_radius = min(input.border_radius, vec4(min(input.scale.x, input.scale.y) / 2.0));

    var transform: mat4x4<f32> = mat4x4<f32>(
        vec4<f32>(scale.x + 1.0, 0.0, 0.0, 0.0),
        vec4<f32>(0.0, scale.y + 1.0, 0.0, 0.0),
        vec4<f32>(0.0, 0.0, 1.0, 0.0),
        vec4<f32>(pos - vec2<f32>(0.5, 0.5), 0.0, 1.0)
    );

    out.position = globals.transform * transform * vec4<f32>(vertex_position(input.vertex_index), 0.0, 1.0);
    out.color = premultiply(input.color);
    out.border_color = premultiply(input.border_color);
    out.pos = input.pos * globals.scale + pos_snap;
    out.scale = input.scale * globals.scale + scale_snap;
    out.border_radius = border_radius * globals.scale;
    out.border_widths = input.border_widths * globals.scale;
    out.shadow_color = premultiply(input.shadow_color);
    out.shadow_offset = input.shadow_offset * globals.scale;
    out.shadow_blur_radius = input.shadow_blur_radius * globals.scale;
    out.shadow_inset = input.shadow_inset;
    out.shadow_spread_radius = input.shadow_spread_radius * globals.scale;
    out.border_dash = input.border_dash * globals.scale;

    return out;
}

@fragment
fn solid_fs_main(
    input: SolidVertexOutput
) -> @location(0) vec4<f32> {
    var dist = rounded_box_sdf(
        -(input.position.xy - input.pos - input.scale * 0.5) * 2.0,
        input.scale,
        input.border_radius * 2.0
    ) / 2.0;

    let bw = input.border_widths; // [top, right, bottom, left]
    let max_border_width = max(max(bw.x, bw.y), max(bw.z, bw.w));

    // Uniform borders can reuse the outer distance for their inner edge.
    let all_equal = bw.x == bw.y && bw.y == bw.z && bw.z == bw.w;

    var padding_dist = dist;
    if max_border_width > 0.0 {
        if all_equal {
            padding_dist = dist + bw.x;
        } else {
            // The border region is between the outline and the padding edge.
            padding_dist = padding_edge_distance(
                input.position.xy,
                input.pos,
                input.scale,
                input.border_radius,
                bw
            );
        }
    }

    // An inset shadow goes over the background and under the border.
    var fill = input.color;
    if input.shadow_color.a > 0.0 && bool(input.shadow_inset) {
        let band = inset_shadow_band(
            input.position.xy,
            dist,
            padding_dist,
            input.pos + vec2(bw.w, bw.x),
            input.pos + input.scale - vec2(bw.y, bw.z),
            max(input.border_radius - vec4(bw.w, bw.y, bw.y, bw.w), vec4(0.0)),
            max(input.border_radius - vec4(bw.x, bw.x, bw.z, bw.z), vec4(0.0)),
            input.shadow_offset,
            input.shadow_blur_radius,
            input.shadow_spread_radius
        );
        fill = inset_shadow_over(fill, input.shadow_color, band);
    }

    var mixed_color = fill;
    if max_border_width > 0.0 {
        // A dashed border shows the fill in its gaps.
        let dash = dash_coverage(input.position.xy, input.pos, input.scale, input.border_radius, input.border_dash);

        // Where inner and outer edges coincide (0-width sides), both coverages
        // cancel out, producing no border artifact.
        mixed_color = mix(fill, input.border_color, border_fraction(dist, padding_dist) * dash);
    }

    let quad_alpha = edge_coverage(dist);
    let quad_color = mixed_color * quad_alpha;

    // Trim the fragment (fill + shadow) to the layer's rounded clip.
    let clip_a = layer_clip_alpha(input.position.xy);

    // An outset shadow is only painted outside the box.
    if input.shadow_color.a > 0.0 && !bool(input.shadow_inset) && quad_alpha < 1.0 {
        let shadow_alpha = outset_shadow_alpha(
            input.position.xy,
            input.pos,
            input.scale,
            input.border_radius,
            input.shadow_offset,
            input.shadow_blur_radius,
            input.shadow_spread_radius
        );

        return mix(quad_color, input.shadow_color, (1.0 - quad_alpha) * shadow_alpha) * clip_a;
    }

    return quad_color * clip_a;
}
