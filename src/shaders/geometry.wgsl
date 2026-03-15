// Geometry pass: diffuse * lightmap
struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) diffuse_uv: vec2<f32>,
    @location(2) lightmap_uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) diffuse_uv: vec2<f32>,
    @location(1) lightmap_uv: vec2<f32>,
};

@group(0) @binding(0) var<uniform> view_proj: mat4x4<f32>;
@group(0) @binding(1) var diffuse_tex: texture_2d<f32>;
@group(0) @binding(2) var diffuse_sampler: sampler;
@group(0) @binding(3) var lightmap_tex: texture_2d<f32>;
@group(0) @binding(4) var lightmap_sampler: sampler;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = view_proj * vec4<f32>(in.position, 1.0);
    out.diffuse_uv = in.diffuse_uv;
    out.lightmap_uv = in.lightmap_uv;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let diffuse = textureSample(diffuse_tex, diffuse_sampler, in.diffuse_uv);
    let lightmap = textureSample(lightmap_tex, lightmap_sampler, in.lightmap_uv);
    // GoldSrc lightmaps are stored at ~1/4 of display brightness.
    // Scale by 4 and add a small ambient floor so geometry is visible.
    let lit = clamp(lightmap.rgb * 4.0 + vec3<f32>(0.05), vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(diffuse.rgb * lit, 1.0);
}
