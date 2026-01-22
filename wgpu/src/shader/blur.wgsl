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
    var total_weight = 0.0;
    
    // Box blur: equal weight for all samples within the box
    for (var i = -half; i <= half; i++) {
        let offset = f32(i);
        let offset_uv = dir * pixel_size * offset;
        color += textureSample(u_texture, u_sampler, uv + offset_uv);
        total_weight += 1.0;
    }
    
    var final_color = color / total_weight;
    
    // Apply border radius clipping (only on final pass, when border_radius is non-zero)
    let has_border_radius = u_uniforms.border_radius.x > 0.0 || u_uniforms.border_radius.y > 0.0 ||
                            u_uniforms.border_radius.z > 0.0 || u_uniforms.border_radius.w > 0.0;
    
    if (has_border_radius) {
        // Use clip_bounds (original widget bounds) for SDF clipping
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
        
        let corner_radius = get_corner_radius(pos, half_size, u_uniforms.border_radius);
        let dist = rounded_rect_sdf(pos, half_size, corner_radius);
        let alpha = 1.0 - smoothstep(-0.5, 0.5, dist);
        
        final_color.a = alpha;
    }
    
    return final_color;

    /*
    // Texture dimensions from uniforms (textures are exact-match with framebuffer)
    let tex_width = u_uniforms.params.z;
    let tex_height = u_uniforms.params.w;

    // Convert framebuffer pixel → normalized UV
    let uv = vec2<f32>(
        frag_pos.x / tex_width,
        frag_pos.y / tex_height
    );

    let radius = u_uniforms.params.x;
    let direction = u_uniforms.params.y;

    // If radius is 0 or very small, just sample directly (passthrough)
    if (radius < 1.0) {
        return textureSample(u_texture, u_sampler, uv);
    }

    let pixel_size = vec2<f32>(1.0 / tex_width, 1.0 / tex_height);
    
    // Direction vector: horizontal (1,0) or vertical (0,1)
    let dir = select(vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), direction < 0.5);
    
    // Sigma is typically radius/3 for a good Gaussian falloff
    let sigma = max(radius / 3.0, 1.0);
    
    // Sample every pixel within the radius for smooth results
    // We limit samples to avoid excessive GPU load
    let max_samples = 64;
    let sample_count = min(i32(ceil(radius)), max_samples);
    
    // Accumulate weighted samples
    var color = vec4<f32>(0.0);
    var total_weight = 0.0;
    
    // Center sample
    let center_weight = gaussian(0.0, sigma);
    color += textureSample(u_texture, u_sampler, uv) * center_weight;
    total_weight += center_weight;
    
    // Symmetric samples on both sides
    for (var i = 1; i <= sample_count; i++) {
        let offset_pixels = f32(i);
        let weight = gaussian(offset_pixels, sigma);
        let offset = dir * pixel_size * offset_pixels;
        
        color += textureSample(u_texture, u_sampler, uv + offset) * weight;
        color += textureSample(u_texture, u_sampler, uv - offset) * weight;
        total_weight += weight * 2.0;
    }
    
    // Normalize
    var final_color = color / total_weight;
    
    // Apply border radius clipping
    // Convert bounds from normalized to pixel coordinates
    let bounds_px = vec4<f32>(
        u_uniforms.bounds.x * tex_width,
        u_uniforms.bounds.y * tex_height,
        u_uniforms.bounds.z * tex_width,
        u_uniforms.bounds.w * tex_height
    );
    
    // Calculate position relative to rectangle center
    let rect_center = vec2<f32>(
        bounds_px.x + bounds_px.z * 0.5,
        bounds_px.y + bounds_px.w * 0.5
    );
    let half_size = vec2<f32>(bounds_px.z * 0.5, bounds_px.w * 0.5);
    let pos = vec2<f32>(frag_pos.x, frag_pos.y) - rect_center;
    
    // Get the appropriate corner radius and compute SDF
    let corner_radius = get_corner_radius(pos, half_size, u_uniforms.border_radius);
    let dist = rounded_rect_sdf(pos, half_size, corner_radius);
    
    // Smooth edge (anti-aliasing)
    let alpha = 1.0 - smoothstep(-0.5, 0.5, dist);
    
    // Apply alpha for rounded corners
    final_color.a *= alpha;
    
    return final_color;
    */
}
