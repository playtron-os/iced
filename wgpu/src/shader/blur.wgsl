struct Uniforms {
    // quad_bounds = (x, y, width, height) in normalized screen coords - expanded for blur sampling
    quad_bounds: vec4<f32>,
    // clip_bounds = (x, y, width, height) in normalized screen coords - original widget bounds for SDF
    clip_bounds: vec4<f32>,
    // params.x = blur_radius, params.y = direction (0=horizontal, 1=vertical)
    // params.z = texture_width, params.w = texture_height
    params: vec4<f32>,
    // border_radius = (top_left, top_right, bottom_right, bottom_left) in pixels
    border_radius: vec4<f32>,
    // fade_params.x = fade_start (0.0–1.0 fraction of bounds height)
    // fade_params.y = 1.0 on the restore pass (emit the complementary weight)
    // fade_params.z = region alpha (1.0 = full strength). Must be 1.0 on any
    //                 pass outside the erase/restore/blur crossfade, or that
    //                 pass renders nothing.
    // fade_params.w = reserved
    fade_params: vec4<f32>,
    // filter_params.x = CSS saturate() amount (1.0 = identity)
    // filter_params.y = 1.0 when the render target format is *Srgb
    // filter_params.z/w = reserved
    filter_params: vec4<f32>,
}

@group(0) @binding(1)
var<uniform> u_uniforms: Uniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let uvs = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0)
    );

    let uv = uvs[vertex_index];

    // Use quad_bounds for vertex positioning (expanded area)
    let x = u_uniforms.quad_bounds.x + uv.x * u_uniforms.quad_bounds.z;
    let y = u_uniforms.quad_bounds.y + uv.y * u_uniforms.quad_bounds.w;

    let clip_x = x * 2.0 - 1.0;
    let clip_y = 1.0 - y * 2.0;

    var out: VertexOutput;
    out.position = vec4<f32>(clip_x, clip_y, 0.0, 1.0);
    return out;
}

@group(0) @binding(0)
var u_sampler: sampler;

@group(1) @binding(0)
var u_texture: texture_2d<f32>;

// Compute signed distance to a rounded rectangle
// pos: position relative to rectangle center
// half_size: half of rectangle width/height
// radius: corner radius for this quadrant
fn rounded_rect_sdf(pos: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let q = abs(pos) - half_size + vec2<f32>(radius);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0))) - radius;
}

// Get the appropriate corner radius based on which quadrant the pixel is in
fn get_corner_radius(pos: vec2<f32>, half_size: vec2<f32>, radii: vec4<f32>) -> f32 {
    // radii = (top_left, top_right, bottom_right, bottom_left)
    if (pos.x < 0.0) {
        if (pos.y < 0.0) {
            return radii.x; // top-left
        } else {
            return radii.w; // bottom-left
        }
    } else {
        if (pos.y < 0.0) {
            return radii.y; // top-right
        } else {
            return radii.z; // bottom-right
        }
    }
}

// ---------------------------------------------------------------------------
// CSS saturate()
//
// Filter Effects L1 defines saturate(N) as feColorMatrix type="saturate", whose
// 3x3 is a lerp between the luma-flattening rank-1 matrix and the identity:
//
//     M(N) = (1 - N) * L + N * I,   every row of L = (0.213, 0.715, 0.072)
//
// so mix(vec3(luma), rgb, N) is that matrix exactly, not an approximation of it.
// ---------------------------------------------------------------------------

// Verbatim from the spec matrix: BT.709 primaries rounded to three decimals.
// NOT Rec.601's 0.299/0.587/0.114, which is a different and visibly wrong set.
const CSS_SATURATE_LUMA: vec3<f32> = vec3<f32>(0.213, 0.715, 0.072);

// IEC 61966-2-1, needed only when the target is an *Srgb format: there the
// hardware has already decoded the sample to linear light, and CSS filters are
// specified on sRGB-encoded values.
fn blur_linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let safe = max(c, vec3<f32>(0.0));
    let lo = safe * 12.92;
    let hi = 1.055 * pow(safe, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, safe <= vec3<f32>(0.0031308));
}

fn blur_srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let safe = max(c, vec3<f32>(0.0));
    let lo = safe / 12.92;
    let hi = pow((safe + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, safe <= vec3<f32>(0.04045));
}

// premul:    premultiplied RGBA, as sampled from and written back to the target
// amount:    CSS saturate() amount (1.0 identity, 0.0 greyscale, >1 super)
// is_linear: 1.0 when the target format is *Srgb, 0.0 for a plain UNORM
fn css_saturate(premul: vec4<f32>, amount: f32, is_linear: f32) -> vec4<f32> {
    let a = premul.a;

    if (is_linear < 0.5) {
        // Plain UNORM target — what the `web-colors` configuration produces.
        // The sample is already sRGB-encoded, which is the space CSS filters
        // are defined in, so there is no transfer to do. With no transfer the
        // whole operation is linear in alpha and the spec's un-premultiply
        // collapses:
        //     M(a*c)             == a * M(c)              (M is a pure 3x3)
        //     clamp(a*x, 0.0, a) == a * clamp(x, 0.0, 1.0) (a >= 0)
        // so clamping the premultiplied value is identical to
        // un-premultiply -> saturate -> clamp -> re-premultiply, minus a
        // divide. The ceiling is `a`, NOT 1.0 — clamping to 1.0 lets a channel
        // exceed its own alpha and shifts hue in the highlights.
        let luma_p = dot(premul.rgb, CSS_SATURATE_LUMA);
        let sat_p = mix(vec3<f32>(luma_p), premul.rgb, amount);
        return vec4<f32>(clamp(sat_p, vec3<f32>(0.0), vec3<f32>(a)), a);
    }

    // *Srgb target: the sample is linear light, and the sRGB transfer does not
    // commute with the alpha scale, so here the un-premultiply is mandatory.
    if (a < 1.0e-5) {
        return premul;
    }
    var rgb = clamp(premul.rgb / a, vec3<f32>(0.0), vec3<f32>(1.0));
    rgb = blur_linear_to_srgb(rgb);
    let luma = dot(rgb, CSS_SATURATE_LUMA);
    rgb = mix(vec3<f32>(luma), rgb, amount);
    rgb = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    rgb = blur_srgb_to_linear(rgb);
    return vec4<f32>(rgb * a, a);
}

// Compute Gaussian weight for a given offset and sigma
fn gaussian(x: f32, sigma: f32) -> f32 {
    let coeff = 1.0 / (sqrt(2.0 * 3.14159265) * sigma);
    let exponent = -(x * x) / (2.0 * sigma * sigma);
    return coeff * exp(exponent);
}

@fragment
fn fs_main(
    @builtin(position) frag_pos: vec4<f32>
) -> @location(0) vec4<f32> {
    // Texture dimensions from uniforms
    let tex_width = u_uniforms.params.z;
    let tex_height = u_uniforms.params.w;
    let radius = u_uniforms.params.x;
    let direction = u_uniforms.params.y; // 0 = horizontal, 1 = vertical

    // Compute SDF alpha for clip_bounds (original widget bounds).
    let has_border_radius = u_uniforms.border_radius.x > 0.0 || u_uniforms.border_radius.y > 0.0 ||
                            u_uniforms.border_radius.z > 0.0 || u_uniforms.border_radius.w > 0.0;

    let bounds_px = vec4<f32>(
        u_uniforms.clip_bounds.x * tex_width,
        u_uniforms.clip_bounds.y * tex_height,
        u_uniforms.clip_bounds.z * tex_width,
        u_uniforms.clip_bounds.w * tex_height
    );

    let rect_center = vec2<f32>(
        bounds_px.x + bounds_px.z * 0.5,
        bounds_px.y + bounds_px.w * 0.5
    );
    let half_size = vec2<f32>(bounds_px.z * 0.5, bounds_px.w * 0.5);
    let pos = vec2<f32>(frag_pos.x, frag_pos.y) - rect_center;

    let corner_radius = select(0.0, get_corner_radius(pos, half_size, u_uniforms.border_radius), has_border_radius);
    let dist = rounded_rect_sdf(pos, half_size, corner_radius);
    var sdf_alpha = 1.0 - smoothstep(-0.5, 0.5, dist);

    // Crossfade weight for this pass = vertical fade x region alpha.
    //
    // The erase pass clears the SDF region, the restore pass additively adds
    // the ORIGINAL scene at (1 - weight) and the final blur pass adds the
    // blurred scene at (weight), so the two always sum to exactly 1 inside the
    // SDF. Driving the weight with the region's opacity therefore gives an
    // exact crossfade between "no filter" and "full filter": at alpha 1 the
    // output is what it always was, and at alpha 0 the region is a pixel-exact
    // no-op rather than a hole.
    //
    // The inversion sits outside the fade_start guard on purpose — the restore
    // pass has to work when the only thing being faded is the region alpha.
    var weight = 1.0;
    let fade_start = u_uniforms.fade_params.x;
    if (fade_start < 1.0) {
        let bounds_top_px = u_uniforms.clip_bounds.y * tex_height;
        let bounds_height_px = u_uniforms.clip_bounds.w * tex_height;
        let local_y = (frag_pos.y - bounds_top_px) / bounds_height_px;
        weight = 1.0 - smoothstep(fade_start, 1.0, local_y);
    }
    weight = weight * u_uniforms.fade_params.z;
    if (u_uniforms.fade_params.y > 0.5) {
        weight = 1.0 - weight;
    }
    sdf_alpha = sdf_alpha * weight;

    // Erase mode (radius < 0): output pure SDF alpha for destination-out blending.
    // The erase pipeline uses blend: src*0 + dst*(1-src_alpha), so only alpha matters.
    // This clears the target content inside the SDF bounds, preventing sharp
    // unblurred content from bleeding through the subsequent blur draw pass.
    if (radius < 0.0) {
        return vec4<f32>(0.0, 0.0, 0.0, sdf_alpha);
    }

    // Convert framebuffer pixel → normalized UV
    let uv = vec2<f32>(
        frag_pos.x / tex_width,
        frag_pos.y / tex_height
    );

    let pixel_size = vec2<f32>(1.0 / tex_width, 1.0 / tex_height);
    
    // Direction vector for separable blur
    let dir = select(vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), direction < 0.5);
    
    // CSS blur() specifies the value as standard deviation (sigma) directly
    // See: https://www.w3.org/TR/filter-effects-1/#funcdef-filter-blur
    // 
    // The W3C spec recommends using three successive box-blurs for sigma >= 2.0:
    // "Three successive box-blurs build a piece-wise quadratic convolution kernel,
    //  which approximates the Gaussian kernel to within roughly 3%."
    //
    // Box size formula from spec: d = floor(sigma * 3 * sqrt(2π) / 4 + 0.5)
    // This simplifies to approximately: d ≈ sigma * 1.8799 ≈ sigma * 1.88
    // 
    // IMPORTANT: d is the TOTAL box width, not the radius!
    // So we sample from -(d-1)/2 to +(d-1)/2, which gives d total samples.
    let sigma = max(radius, 1.0);
    
    // Calculate box size per W3C formula: d = floor(sigma * 3 * sqrt(2π) / 4 + 0.5)
    // sqrt(2π) ≈ 2.5066, so 3 * 2.5066 / 4 ≈ 1.88
    let d = i32(floor(sigma * 1.8799 + 0.5));
    let box_size = max(d, 1);
    
    // Half-width for sampling: sample from -half to +half (total of box_size samples when odd)
    let half = (box_size - 1) / 2;
    
    var color = vec4<f32>(0.0);
    var total_weight: f32 = 0.0;

    // Center sample
    color += textureSample(u_texture, u_sampler, uv);
    total_weight += 1.0;

    // Bilinear-optimized box blur: pair adjacent texels via hardware
    // interpolation.  A fetch at half-offset (i+0.5) returns the average
    // of texels at i and i+1, so multiply by 2 to recover their sum
    // (box weight = 1 each).  This halves texture fetches vs per-texel
    // sampling.
    var i: i32 = 1;
    loop {
        if (i + 1 > half) { break; }
        let tap_offset = f32(i) + 0.5;
        let step = dir * pixel_size * tap_offset;
        color += (textureSample(u_texture, u_sampler, uv + step)
                + textureSample(u_texture, u_sampler, uv - step)) * 2.0;
        total_weight += 4.0;
        i += 2;
    }

    // Unpaired edge sample when per-side count is odd
    if (i <= half) {
        let step = dir * pixel_size * f32(i);
        color += textureSample(u_texture, u_sampler, uv + step)
               + textureSample(u_texture, u_sampler, uv - step);
        total_weight += 2.0;
    }

    let final_color = color / total_weight;

    // CSS applies a backdrop-filter list in order, so `blur(R) saturate(N)`
    // saturates the finished blur — exactly once. Only one uniform block ever
    // carries a non-identity amount: the final vertical pass. The erase pass
    // returned above; the restore pass, the five intermediate ping-pong passes
    // and both blits carry 1.0 and short-circuit here. The epsilon guard is
    // what makes identity bit-exact whatever the driver does with FMA.
    let saturation = u_uniforms.filter_params.x;
    var out_color = final_color;
    if (abs(saturation - 1.0) > 0.001) {
        out_color = css_saturate(final_color, saturation, u_uniforms.filter_params.y);
    }

    // Scale the premultiplied blur result by SDF alpha.
    // The erase pass already cleared the target, so premultiplied alpha blending
    // writes the blur result directly: src + dst*(1-src_alpha) = src + 0 = src.
    // Where content was opaque, blur_alpha ≈ 1 → compositor sees opaque blurred content.
    // Where content was transparent, blur_alpha ≈ 0 → compositor blur shows through.
    return out_color * sdf_alpha;
}
