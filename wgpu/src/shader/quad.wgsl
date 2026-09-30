struct Globals {
    transform: mat4x4<f32>,
    scale: f32,
    // Rounded clip applied to the whole layer (physical px). `clip_bounds` is
    // [x, y, w, h]; `clip_radius` is per-corner. Ordinary layers pass a huge
    // rectangle with zero radius, so `layer_clip_alpha` stays 1.0 everywhere.
    clip_bounds: vec4<f32>,
    clip_radius: vec4<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;

fn rounded_box_sdf(p: vec2<f32>, size: vec2<f32>, corners: vec4<f32>) -> f32 {
    var box_half = select(corners.yz, corners.xw, p.x > 0.0);
    var corner = select(box_half.y, box_half.x, p.y > 0.0);
    var q = abs(p) - size + corner;
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2(0.0))) - corner;
}

fn edge_coverage(distance: f32) -> f32 {
    return clamp(0.5 - distance, 0.0, 1.0);
}

// The inner and outer edges partition a pixel; their coverages are correlated.
// Multiplying outer * (1 - inner) overcounts a subpixel stroke, changing its
// weight as a corner crosses the pixel grid. Normalize only for the material
// mix: the caller applies the outer shape's coverage exactly once afterwards.
fn stroke_coverage(outer_distance: f32, inner_distance: f32) -> f32 {
    return max(edge_coverage(outer_distance) - edge_coverage(inner_distance), 0.0);
}

// Border widths in physical pixels, snapped as CSS snaps a border width to
// device pixels: down to a whole pixel, but a border thinner than a pixel is
// one pixel wide rather than a faint anti-aliased line.
fn snap_border_widths(widths: vec4<f32>) -> vec4<f32> {
    let hairline = widths > vec4(0.0) & widths < vec4(1.0);
    return select(floor(widths + vec4(1.0e-5)), vec4(1.0), hairline);
}

// Signed distance from `frag_pos` to the padding edge of the box at `pos` with
// `size`: inset by `widths` [top, right, bottom, left], each corner shrunk on
// each axis by the width of the side it meets, as CSS does, so a corner between
// sides of different widths is a quarter ellipse. Exact along the sides; at a
// corner, a first-order estimate that is exact on the curve itself.
fn padding_edge_distance(frag_pos: vec2<f32>, pos: vec2<f32>, size: vec2<f32>, radius: vec4<f32>, widths: vec4<f32>) -> f32 {
    let half = max(size - vec2(widths.w + widths.y, widths.x + widths.z), vec2(0.0)) * 0.5;
    let p = frag_pos - pos - vec2(widths.w, widths.x) - half;

    // The nearest corner's radius and the widths of the sides it joins.
    var corner: f32;
    var sides: vec2<f32>;
    if p.x < 0.0 {
        corner = select(radius.x, radius.w, p.y > 0.0);
        sides = vec2(widths.w, select(widths.x, widths.z, p.y > 0.0));
    } else {
        corner = select(radius.y, radius.z, p.y > 0.0);
        sides = vec2(widths.y, select(widths.x, widths.z, p.y > 0.0));
    }

    let r = max(vec2(corner) - sides, vec2(0.0));
    let q = abs(p) - half + r;

    if r.x > 0.0 && r.y > 0.0 && q.x > 0.0 && q.y > 0.0 {
        let k0 = length(q / r);
        let k1 = length(q / (r * r));
        return k0 * (k0 - 1.0) / k1;
    }

    let d = abs(p) - half;
    return min(max(d.x, d.y), 0.0) + length(max(d, vec2(0.0)));
}

fn border_fraction(outer_distance: f32, inner_distance: f32) -> f32 {
    return stroke_coverage(outer_distance, inner_distance) / max(edge_coverage(outer_distance), 0.001);
}

// Both materials are premultiplied, but not yet masked by the quad's edge.
// An inset shadow/highlight goes OVER the fill; interpolating towards its RGBA
// instead would replace opaque artwork with a translucent band. Apply the
// shared shape mask once, so an opaque tile retains its antialiased silhouette.
fn inset_shadow_over(fill: vec4<f32>, shadow: vec4<f32>, outer_distance: f32, hole_distance: f32, blur: f32) -> vec4<f32> {
    let outer = edge_coverage(outer_distance);
    var band: f32;
    if blur <= 0.0 {
        // A sharp inset is the part of the body outside the translated hole.
        // In particular, coincident edges cancel rather than casting a halo.
        band = stroke_coverage(outer_distance, hole_distance);
    } else {
        // Retain at least a physical pixel of AA at the blurred hole boundary.
        let extent = max(blur, 0.5);
        band = outer * smoothstep(-extent, extent, hole_distance);
    }
    return fill * outer + (shadow - fill * shadow.a) * band;
}

fn outset_shadow_alpha(distance: f32, blur: f32) -> f32 {
    // smoothstep with equal edges is undefined (not a hard-edged shadow).
    if blur <= 0.0 {
        return edge_coverage(distance);
    }
    return 1.0 - smoothstep(-blur, blur, max(distance, 0.0));
}

// Coverage (1 = keep, 0 = discard) of a fragment under the layer's rounded
// clip, with the same AA convention as the quad fill so trimmed corners stay
// smooth. `frag_pos` is the fragment's physical-pixel position.
fn layer_clip_alpha(frag_pos: vec2<f32>) -> f32 {
    let center = globals.clip_bounds.xy + globals.clip_bounds.zw * 0.5;
    let dist = rounded_box_sdf(
        -(frag_pos - center) * 2.0,
        globals.clip_bounds.zw,
        globals.clip_radius * 2.0
    ) / 2.0;
    return clamp(0.5 - dist, 0.0, 1.0);
}

// How far along a rounded rectangle's outline a fragment sits, clockwise from
// the end of the top-left corner. `p` is the fragment relative to the top-left
// of a `size` box; `radius` is per-corner [tl, tr, br, bl].
fn outline_position(p: vec2<f32>, size: vec2<f32>, radius: vec4<f32>) -> f32 {
    let quarter = 1.5707963;
    let top = size.x - radius.x - radius.y;
    let right = size.y - radius.y - radius.z;
    let bottom = size.x - radius.z - radius.w;
    let left = size.y - radius.w - radius.x;

    // Corner boxes first: the arc's angle, measured clockwise on screen.
    if p.x < radius.x && p.y < radius.x {
        var a = atan2(p.y - radius.x, p.x - radius.x);
        if a < 0.0 { a = a + 6.2831853; }
        let t = clamp((a - 3.1415927) / quarter, 0.0, 1.0);
        return top + radius.y * quarter + right + radius.z * quarter + bottom + radius.w * quarter + left + t * radius.x * quarter;
    }
    if p.x > size.x - radius.y && p.y < radius.y {
        let a = atan2(p.y - radius.y, p.x - (size.x - radius.y));
        let t = clamp((a + quarter) / quarter, 0.0, 1.0);
        return top + t * radius.y * quarter;
    }
    if p.x > size.x - radius.z && p.y > size.y - radius.z {
        let a = atan2(p.y - (size.y - radius.z), p.x - (size.x - radius.z));
        let t = clamp(a / quarter, 0.0, 1.0);
        return top + radius.y * quarter + right + t * radius.z * quarter;
    }
    if p.x < radius.w && p.y > size.y - radius.w {
        let a = atan2(p.y - (size.y - radius.w), p.x - radius.w);
        let t = clamp((a - quarter) / quarter, 0.0, 1.0);
        return top + radius.y * quarter + right + radius.z * quarter + bottom + t * radius.w * quarter;
    }

    // Otherwise the nearest straight edge.
    let d = vec4<f32>(p.y, size.x - p.x, size.y - p.y, p.x);
    let nearest = min(min(d.x, d.y), min(d.z, d.w));
    if nearest == d.x {
        return p.x - radius.x;
    }
    if nearest == d.y {
        return top + radius.y * quarter + (p.y - radius.y);
    }
    if nearest == d.z {
        return top + radius.y * quarter + right + radius.z * quarter + (size.x - radius.z - p.x);
    }
    return top + radius.y * quarter + right + radius.z * quarter + bottom + radius.w * quarter + (size.y - radius.w - p.y);
}

// Coverage of a dashed border at `frag_pos`: 1 on a dash, 0 in a gap, with a
// one-pixel ramp between. A zero pattern is solid.
fn dash_coverage(frag_pos: vec2<f32>, pos: vec2<f32>, size: vec2<f32>, radius: vec4<f32>, dash: vec2<f32>) -> f32 {
    let period = dash.x + dash.y;
    if dash.x <= 0.0 || period <= 0.0 {
        return 1.0;
    }
    let s = outline_position(frag_pos - pos, size, radius);
    let u = s - floor(s / period) * period;
    return clamp(min(u, dash.x - u) + 0.5, 0.0, 1.0);
}
