// A ball of light with a starburst, on a quad facing the camera.
fn pipeline_fragment(input: PipelineInput) -> vec4<f32> {
    let p = input.uv;
    let r = length(p);
    if (r > 1.0) {
        return vec4<f32>(0.0);
    }
    let core = exp(-r * r * 18.0) * 4.0;
    let halo = exp(-r * 4.0) * 0.9;
    let spikes = exp(-abs(p.x) * 30.0) * exp(-abs(p.y) * 2.5) + exp(-abs(p.y) * 30.0) * exp(-abs(p.x) * 2.5);
    let edge = 1.0 - smoothstep(0.7, 1.0, r);
    let col = mix(input.color.rgb, vec3<f32>(1.0, 0.97, 0.92), exp(-r * 6.0));
    return vec4<f32>(col * (core + halo + spikes * 0.8) * edge * input.color.a, 1.0);
}
