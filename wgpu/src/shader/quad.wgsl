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
