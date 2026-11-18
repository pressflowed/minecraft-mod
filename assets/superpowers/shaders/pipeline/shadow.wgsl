// A black hole's shadow and photon ring, on a quad facing the camera (drawn over the picture by
// its alpha): black inside 0.55, a thin bright ring at its edge, a violet haze outside.
fn pipeline_fragment(input: PipelineInput) -> vec4<f32> {
    let r = length(input.uv);
    if (r > 1.0) {
        return vec4<f32>(0.0);
    }
    let core = 1.0 - smoothstep(0.52, 0.56, r);
    let x = (r - 0.585) / 0.018;
    let ring = exp(-x * x);
    let haze = select(0.0, exp(-(r - 0.585) / 0.12) * 0.45, r > 0.585);
    let col = vec3<f32>(1.0, 0.86, 0.62) * ring * 3.0 + vec3<f32>(0.5, 0.18, 0.95) * haze;
    let alpha = clamp(core + ring + haze * 0.5, 0.0, 1.0) * input.color.a;
    return vec4<f32>(col, alpha);
}
