@group(0) @binding(0) var kept: texture_2d<f32>;

@vertex
fn vs_copy(@builtin(vertex_index) corner: u32) -> @builtin(position) vec4<f32> {
    let at = vec2<f32>(f32((corner << 1u) & 2u), f32(corner & 2u));
    return vec4<f32>(at * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_copy(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(kept, vec2<i32>(position.xy), 0);
}
