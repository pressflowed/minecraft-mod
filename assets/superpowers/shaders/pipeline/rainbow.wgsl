// The dash's trail: a band of every colour, uv.x along it from 0 to 1, uv.y across.
fn hue(h: f32) -> vec3<f32> {
    return clamp(abs(fract(h + vec3<f32>(0.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0 - 3.0) - 1.0, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn pipeline_fragment(input: PipelineInput) -> vec4<f32> {
    let y = input.uv.y;
    let band = exp(-y * y * 2.5);
    let core = exp(-y * y * 30.0) * 0.8;
    let ghosts = 0.75 + 0.25 * sin(input.uv.x * 24.0 - frame.time * 18.0);
    let col = (hue(input.uv.x * 1.3 - frame.time * 0.8) * band + vec3<f32>(core)) * ghosts * 1.6;
    return vec4<f32>(col * input.color.rgb * input.color.a, 1.0);
}
