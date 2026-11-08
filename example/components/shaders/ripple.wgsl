// Ripple effect for iced.shader. Set `animated = true` to advance riced.time.
//
// extraArgs from Lua:
//   opacity - overall alpha (0..1)
//   speed   - animation speed multiplier
@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let aspect = riced.resolution.x / max(riced.resolution.y, 1.0);
    let p = (uv - vec2<f32>(0.5)) * vec2<f32>(aspect, 1.0);
    let distance = length(p);
    let time = riced.time * riced.speed;

    let wave = sin(distance * 36.0 - time * 5.0) * 0.5 + 0.5;
    let ring = smoothstep(0.15, 1.0, wave);
    let fade = 1.0 - smoothstep(0.35, 0.75, distance);
    let glow = exp(-distance * 4.0) * 0.35;

    let color = mix(
        vec3<f32>(0.15, 0.55, 1.0),
        vec3<f32>(0.95, 0.35, 0.75),
        ring,
    );
    let alpha = clamp((ring * 0.6 + glow) * fade * riced.opacity, 0.0, 1.0);
    return vec4<f32>(color, alpha);
}
