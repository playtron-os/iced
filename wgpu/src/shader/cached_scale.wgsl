// Cached scale shader
// Renders an offscreen texture as a scaled/translated quad, optionally
// blurring it on the way.
//
// The blur here is a CONTENT blur — the CSS `filter: blur()` kind, which
// softens the element itself — not the backdrop blur in blur.wgsl, which
// softens whatever is behind an element. It works because the offscreen
// texture this samples holds only the captured content on transparent black,
// so a plain Gaussian over it fades the content's own edges out correctly
// instead of smearing the scene into them.
//
// The Gaussian is separable and run as two passes over the same shader: a
// horizontal pass into an intermediate texture, then a vertical pass that
// doubles as the composite. Doing it in one 2D pass would cost taps squared.

struct Uniforms {
    // Source region in normalized texture coords (x, y, width, height)
    src_rect: vec4<f32>,
    // Destination quad in clip space: (x, y, width, height) in NDC
    dst_rect: vec4<f32>,
    // Blur: (step_u, step_v, sigma_texels, taps_per_side)
    //
    // `step` is one texel along the axis being blurred, so the same shader
    // serves both passes. `taps_per_side` of 0 means no blur and takes the
    // single-sample path — the cost of the blur is only paid while something
    // is actually blurred.
    blur: vec4<f32>,
}

@group(0) @binding(0) var u_sampler: sampler;
@group(0) @binding(1) var<uniform> u_uniforms: Uniforms;
@group(1) @binding(0) var u_texture: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    // 6 vertices for two triangles forming a quad
    var local_uvs = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0)
    );

    let local_uv = local_uvs[vertex_index];

    // Map local UV to destination clip-space position
    let x = u_uniforms.dst_rect.x + local_uv.x * u_uniforms.dst_rect.z;
    let y = u_uniforms.dst_rect.y + local_uv.y * u_uniforms.dst_rect.w;

    // Map local UV to source texture coordinates
    let tex_u = u_uniforms.src_rect.x + local_uv.x * u_uniforms.src_rect.z;
    let tex_v = u_uniforms.src_rect.y + local_uv.y * u_uniforms.src_rect.w;

    var output: VertexOutput;
    output.position = vec4<f32>(x, y, 0.0, 1.0);
    output.uv = vec2<f32>(tex_u, tex_v);
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let taps = i32(u_uniforms.blur.w);

    if (taps <= 0) {
        return textureSample(u_texture, u_sampler, input.uv);
    }

    let step = vec2<f32>(u_uniforms.blur.x, u_uniforms.blur.y);
    let sigma = max(u_uniforms.blur.z, 0.0001);
    // The 1/(sigma*sqrt(2pi)) factor is left out on purpose: the weights are
    // normalised by their own sum below, which also corrects for the tails
    // past `taps` that the truncated kernel drops. Including it would just be
    // divided straight back out.
    let denom = 2.0 * sigma * sigma;

    // Samples are premultiplied-alpha, which is exactly the space a Gaussian
    // is meant to average in — weighting straight alpha would darken the
    // softened edge toward black.
    var accum = vec4<f32>(0.0);
    var weight_sum = 0.0;

    for (var i = -taps; i <= taps; i = i + 1) {
        let offset = f32(i);
        let weight = exp(-(offset * offset) / denom);
        accum = accum + textureSample(u_texture, u_sampler, input.uv + step * offset) * weight;
        weight_sum = weight_sum + weight;
    }

    return accum / weight_sum;
}
