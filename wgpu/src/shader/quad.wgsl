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

// The fill with an inset shadow over it; the shadow's `band` is a share of the
// quad's coverage, so the quad's edge still masks the result exactly once.
fn inset_shadow_over(fill: vec4<f32>, shadow: vec4<f32>, band: f32) -> vec4<f32> {
    return fill + (shadow - fill * shadow.a) * band;
}

// The standard normal CDF, from Abramowitz and Stegun's erf 7.1.26 (error < 1.5e-7).
fn normal_cdf(x: f32) -> f32 {
    let z = abs(x) * 0.70710678;
    let t = 1.0 / (1.0 + 0.3275911 * z);
    let polynomial = t * (0.254829592 + t * (-0.284496736 + t * (1.421413741 + t * (-1.453152027 + t * 1.061405429))));
    let erf = 1.0 - polynomial * exp(-z * z);
    return 0.5 + 0.5 * select(-erf, erf, x >= 0.0);
}

// How far in from its sides a corner of `radius` (horizontal, vertical) is at
// `depth` below (or above) its edge.
fn corner_inset(radius: vec2<f32>, depth: f32) -> f32 {
    if depth >= radius.y || radius.x <= 0.0 {
        return 0.0;
    }
    let t = 1.0 - depth / radius.y;
    return radius.x * (1.0 - sqrt(max(1.0 - t * t, 0.0)));
}

// The horizontal extent at height `y` of the box from `lo` to `hi`, whose corners
// have horizontal radii `rx` and vertical radii `ry` [tl, tr, br, bl].
fn box_span(y: f32, lo: vec2<f32>, hi: vec2<f32>, rx: vec4<f32>, ry: vec4<f32>) -> vec2<f32> {
    let top = y - lo.y;
    let bottom = hi.y - y;
    let left = max(corner_inset(vec2(rx.x, ry.x), top), corner_inset(vec2(rx.w, ry.w), bottom));
    let right = max(corner_inset(vec2(rx.y, ry.y), top), corner_inset(vec2(rx.z, ry.z), bottom));
    return vec2(lo.x + left, hi.x - right);
}

// The share of the blurred box that rows `first`..`last` give `p`, summed over
// sixteen slices of the rows the Gaussian reaches (three deviations each way).
fn blurred_rows(p: vec2<f32>, first: f32, last: f32, lo: vec2<f32>, hi: vec2<f32>, rx: vec4<f32>, ry: vec4<f32>, sigma: f32) -> f32 {
    let a = max(first, p.y - 3.0 * sigma);
    let b = min(last, p.y + 3.0 * sigma);
    if b <= a {
        return 0.0;
    }

    // Four deviations clear of every span's ends, each row gives all of its
    // Gaussian weight or none.
    let reach = 4.0 * sigma;
    let widest = max(max(rx.x, rx.w), max(rx.y, rx.z));
    if p.x < lo.x - reach || p.x > hi.x + reach {
        return 0.0;
    }
    if p.x > lo.x + widest + reach && p.x < hi.x - widest - reach {
        return normal_cdf((b - p.y) / sigma) - normal_cdf((a - p.y) / sigma);
    }

    let step = (b - a) / 16.0;
    var below = normal_cdf((a - p.y) / sigma);
    var alpha = 0.0;
    for (var i = 1; i <= 16; i++) {
        let above = normal_cdf((a + f32(i) * step - p.y) / sigma);
        let span = box_span(a + (f32(i) - 0.5) * step, lo, hi, rx, ry);
        alpha += (above - below) * (normal_cdf((span.y - p.x) / sigma) - normal_cdf((span.x - p.x) / sigma));
        below = above;
    }
    return alpha;
}

// Coverage at `p` of the box from `lo` to `hi` (corner radii `rx`, `ry`) blurred
// by a Gaussian of standard deviation `sigma`, as CSS blurs a box-shadow. Rows
// clear of the corners are integrated exactly, the rest in slices.
fn blurred_box(p: vec2<f32>, lo: vec2<f32>, hi: vec2<f32>, rx: vec4<f32>, ry: vec4<f32>, sigma: f32) -> f32 {
    if hi.x <= lo.x || hi.y <= lo.y {
        return 0.0;
    }
    let top = max(ry.x, ry.y);
    let bottom = max(ry.z, ry.w);
    if top + bottom >= hi.y - lo.y {
        return blurred_rows(p, lo.y, hi.y, lo, hi, rx, ry, sigma);
    }
    let a = max(lo.y + top, p.y - 3.0 * sigma);
    let b = min(hi.y - bottom, p.y + 3.0 * sigma);
    var straight = 0.0;
    if b > a {
        straight = (normal_cdf((b - p.y) / sigma) - normal_cdf((a - p.y) / sigma))
            * (normal_cdf((hi.x - p.x) / sigma) - normal_cdf((lo.x - p.x) / sigma));
    }
    return straight
        + blurred_rows(p, lo.y, lo.y + top, lo, hi, rx, ry, sigma)
        + blurred_rows(p, hi.y - bottom, hi.y, lo, hi, rx, ry, sigma);
}

// CSS's corner radii for a shadow whose box grows by `outset` (shrinks when
// negative): a corner tighter than the outset grows by less, so a square corner
// stays square.
fn spread_radii(radii: vec4<f32>, outset: f32) -> vec4<f32> {
    if outset <= 0.0 {
        return max(radii + vec4(outset), vec4(0.0));
    }
    let r = min(radii / outset, vec4(1.0)) - vec4(1.0);
    return radii + outset * (vec4(1.0) + r * r * r);
}

// What an outset box-shadow covers at `p`: the box at `pos` with `size` and
// `radii`, moved by `offset`, grown by `spread` and blurred by a Gaussian of
// standard deviation half `blur`.
fn outset_shadow_alpha(p: vec2<f32>, pos: vec2<f32>, size: vec2<f32>, radii: vec4<f32>, offset: vec2<f32>, blur: f32, spread: f32) -> f32 {
    let lo = pos + offset - vec2(spread);
    let hi = pos + size + offset + vec2(spread);
    let corners = spread_radii(radii, spread);
    // A deviation under a quarter pixel is too small to see: draw it sharp.
    if blur < 0.5 {
        if hi.x <= lo.x || hi.y <= lo.y {
            return 0.0;
        }
        return edge_coverage(rounded_box_sdf(-(p - (lo + hi) * 0.5) * 2.0, hi - lo, corners * 2.0) / 2.0);
    }
    return blurred_box(p, lo, hi, corners, corners, blur * 0.5);
}

// How much of an inset box-shadow shows at `p`, as a share of the quad's
// coverage (`outer_distance`). It fills the padding box (`padding_distance`;
// `lo` to `hi` with corner radii `rx`, `ry`) but for a hole: the padding box
// moved by `offset`, shrunk by `spread` and blurred by half `blur`.
fn inset_shadow_band(p: vec2<f32>, outer_distance: f32, padding_distance: f32, lo: vec2<f32>, hi: vec2<f32>, rx: vec4<f32>, ry: vec4<f32>, offset: vec2<f32>, blur: f32, spread: f32) -> f32 {
    let hole_lo = lo + offset + vec2(spread);
    let hole_hi = hi + offset - vec2(spread);
    let hole_rx = spread_radii(rx, -spread);
    let hole_ry = spread_radii(ry, -spread);
    var band: f32;
    if blur < 0.5 {
        // The padding box outside the hole, so coincident edges cancel rather
        // than casting a halo.
        var hole = 1.0e5;
        if hole_hi.x > hole_lo.x && hole_hi.y > hole_lo.y {
            hole = rounded_box_sdf(-(p - (hole_lo + hole_hi) * 0.5) * 2.0, hole_hi - hole_lo, min(hole_rx, hole_ry) * 2.0) / 2.0;
        }
        band = stroke_coverage(padding_distance, hole);
    } else {
        band = edge_coverage(padding_distance) * (1.0 - blurred_box(p, hole_lo, hole_hi, hole_rx, hole_ry, blur * 0.5));
    }
    return min(band / max(edge_coverage(outer_distance), 0.001), 1.0);
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
// of a `size` box; `radius` is per-corner [tl, tr, br, bl]. Corners are
// measured `inset` inside the outline, on a border's centre line, where the
// browser strokes a dash.
fn outline_position(p: vec2<f32>, size: vec2<f32>, radius: vec4<f32>, inset: f32) -> f32 {
    let quarter = 1.5707963;
    let arc = max(radius - vec4(inset), vec4(0.0)) * quarter;
    let top = size.x - radius.x - radius.y;
    let right = size.y - radius.y - radius.z;
    let bottom = size.x - radius.z - radius.w;
    let left = size.y - radius.w - radius.x;

    // Corner boxes first: the arc's angle, measured clockwise on screen.
    if p.x < radius.x && p.y < radius.x {
        var a = atan2(p.y - radius.x, p.x - radius.x);
        if a < 0.0 { a = a + 6.2831853; }
        let t = clamp((a - 3.1415927) / quarter, 0.0, 1.0);
        return top + arc.y + right + arc.z + bottom + arc.w + left + t * arc.x;
    }
    if p.x > size.x - radius.y && p.y < radius.y {
        let a = atan2(p.y - radius.y, p.x - (size.x - radius.y));
        let t = clamp((a + quarter) / quarter, 0.0, 1.0);
        return top + t * arc.y;
    }
    if p.x > size.x - radius.z && p.y > size.y - radius.z {
        let a = atan2(p.y - (size.y - radius.z), p.x - (size.x - radius.z));
        let t = clamp(a / quarter, 0.0, 1.0);
        return top + arc.y + right + t * arc.z;
    }
    if p.x < radius.w && p.y > size.y - radius.w {
        let a = atan2(p.y - (size.y - radius.w), p.x - radius.w);
        let t = clamp((a - quarter) / quarter, 0.0, 1.0);
        return top + arc.y + right + arc.z + bottom + t * arc.w;
    }

    // Otherwise the nearest straight edge.
    let d = vec4<f32>(p.y, size.x - p.x, size.y - p.y, p.x);
    let nearest = min(min(d.x, d.y), min(d.z, d.w));
    if nearest == d.x {
        return p.x - radius.x;
    }
    if nearest == d.y {
        return top + arc.y + (p.y - radius.y);
    }
    if nearest == d.z {
        return top + arc.y + right + arc.z + (size.x - radius.z - p.x);
    }
    return top + arc.y + right + arc.z + bottom + arc.w + (size.y - radius.w - p.y);
}

// Coverage of a dashed border at `frag_pos`: 1 on a dash, 0 in a gap, with a
// one-pixel ramp between. A zero pattern is solid.
fn dash_coverage(frag_pos: vec2<f32>, pos: vec2<f32>, size: vec2<f32>, radius: vec4<f32>, dash: vec2<f32>, inset: f32) -> f32 {
    let period = dash.x + dash.y;
    if dash.x <= 0.0 || period <= 0.0 {
        return 1.0;
    }
    let s = outline_position(frag_pos - pos, size, radius, inset);
    let u = s - floor(s / period) * period;
    return clamp(min(u, dash.x - u) + 0.5, 0.0, 1.0);
}
