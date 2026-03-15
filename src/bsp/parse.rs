// Stub — will be fully implemented in Task 6
use glam::Vec3;
use image::RgbaImage;

pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub sky_index_offset: u32,
    pub diffuse_atlas: RgbaImage,
    pub lightmap_atlas: RgbaImage,
    pub entity_origins: Vec<Vec3>,
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub diffuse_uv: [f32; 2],
    pub lightmap_uv: [f32; 2],
}
