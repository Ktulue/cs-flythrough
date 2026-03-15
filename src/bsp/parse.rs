use std::path::Path;

use anyhow::{Context, Result};
use image::RgbaImage;
// glam re-exported from qbsp is version 0.30; our crate uses glam 0.32.
// We explicitly import from each to avoid ambiguity.
use glam::Vec3;

use qbsp::{
    BspData, BspParseInput, BspParseSettings,
    data::texture::BspSurfaceFlags,
    mesh::lightmap::{
        ComputeLightmapSettings, LightmapUvMap, PerStyleLightmapData, PerStyleLightmapPacker,
    },
};

use crate::bsp::wad::load_textures;

/// GPU vertex — 32 bytes stride, 3 attributes:
/// @location(0): position [f32; 3] — world-space XYZ
/// @location(1): diffuse_uv [f32; 2] — UV into diffuse atlas
/// @location(2): lightmap_uv [f32; 2] — UV into lightmap atlas
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub diffuse_uv: [f32; 2],
    pub lightmap_uv: [f32; 2],
}

pub struct MeshData {
    pub vertices: Vec<Vertex>,
    /// Geometry indices first, then sky indices (starting at sky_index_offset).
    pub indices: Vec<u32>,
    /// Index into `indices` where sky faces begin.
    pub sky_index_offset: u32,
    pub diffuse_atlas: RgbaImage,
    pub lightmap_atlas: RgbaImage,
    /// Unsorted; camera module sorts these.
    pub entity_origins: Vec<Vec3>,
}

/// Parse the worldspawn entity block from the entity lump to find the "wad" key.
///
/// Returns the raw value string (semicolon-separated WAD paths) or None.
fn find_worldspawn_wad<'a>(entity_lump: &'a str) -> Option<&'a str> {
    // Entity blocks delimited by '{' ... '}'
    for block in entity_lump.split('{') {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }
        // Check if this is the worldspawn block.
        let mut is_worldspawn = false;
        let mut wad_value: Option<&str> = None;
        for line in block.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix('"') {
                if let Some((key, after_key)) = rest.split_once('"') {
                    let value = after_key.trim().trim_matches('"');
                    if key == "classname" && value == "worldspawn" {
                        is_worldspawn = true;
                    }
                    if key == "wad" {
                        wad_value = Some(
                            // The value still has the trailing quote stripped via trim_matches above,
                            // but let's be careful — we need the raw value from after the space.
                            after_key.trim().trim_start_matches('"').trim_end_matches('"'),
                        );
                    }
                }
            }
        }
        if is_worldspawn {
            return wad_value;
        }
    }
    None
}

/// Resolve WAD paths from the worldspawn "wad" key against cs_install_path.
///
/// The value format is semicolon-separated absolute paths like:
///   C:\games\cstrike\de_dust.wad;C:\games\cstrike\halflife.wad
///
/// We extract only the filename and probe cs_install_path/cstrike/ and cs_install_path/czero/.
fn resolve_wad_paths(wad_value: &str, cs_install_path: &Path) -> Vec<std::path::PathBuf> {
    let search_dirs = [
        cs_install_path.join("cstrike"),
        cs_install_path.join("czero"),
        cs_install_path.join("valve"),   // halflife.wad, decals, shared textures
    ];

    let mut result = Vec::new();
    for entry in wad_value.split(';') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        // Extract the filename portion from the (possibly Windows-style) path.
        let filename = entry
            .replace('\\', "/")
            .split('/')
            .last()
            .unwrap_or(entry)
            .to_owned();
        if filename.is_empty() {
            continue;
        }
        // Try each search directory; first match wins.
        for dir in &search_dirs {
            let candidate = dir.join(&filename);
            if candidate.exists() {
                result.push(candidate);
                break;
            }
        }
    }
    result
}

/// Determine whether a face is a sky face.
///
/// Sky detection method: BspSurfaceFlags::SKY bit (0x4) for GoldSrc BSP30.
/// Fallback: texture name starts with "sky" (case-insensitive).
/// The SKY flag is checked first (preferred); the name prefix is used as a fallback
/// when the flag is unavailable (e.g. the texture info is missing).
fn is_sky_face(bsp: &BspData, face: &qbsp::data::models::BspFace) -> bool {
    let tex_info = &bsp.tex_info[face.texture_info_idx.0 as usize];

    // Primary: GoldSrc BSP30 exposes SURF_SKY = 0x4 via BspSurfaceFlags::SKY.
    // The `flags.surface_flags` field carries these for BSP30; for BSP29/BSP2 it
    // will be zero, so the fallback texture-name check still covers those formats.
    if tex_info.flags.surface_flags.contains(BspSurfaceFlags::SKY) {
        return true;
    }

    // Fallback: texture name prefix "sky" (case-insensitive).
    if let Some(name) = bsp.get_texture_name(tex_info) {
        if name.as_str().to_ascii_lowercase().starts_with("sky") {
            return true;
        }
    }

    false
}

/// Build the lightmap atlas as an RgbaImage from the PerStyleLightmapData output.
///
/// We pick the NORMAL style lightmap and convert from RgbImage to RgbaImage.
/// If no NORMAL style exists (unlikely), we use the first available style.
fn style_data_to_rgba(data: &PerStyleLightmapData) -> RgbaImage {
    use qbsp::data::lighting::LightmapStyle;

    let inner = data.inner();
    let rgb = inner
        .get(&LightmapStyle::NORMAL)
        .or_else(|| inner.values().next());

    match rgb {
        Some(rgb_img) => {
            let (w, h) = (rgb_img.width(), rgb_img.height());
            let mut rgba = RgbaImage::new(w, h);
            for (x, y, px) in rgba.enumerate_pixels_mut() {
                let src = rgb_img.get_pixel(x, y);
                *px = image::Rgba([src[0], src[1], src[2], 255]);
            }
            rgba
        }
        None => RgbaImage::new(1, 1),
    }
}

/// Load and parse a GoldSrc BSP file, returning a `MeshData` ready for GPU upload.
pub fn load(bsp_path: &Path, cs_install_path: &Path) -> Result<MeshData> {
    // ── 1. Read and parse the BSP ────────────────────────────────────────────
    let bsp_bytes = std::fs::read(bsp_path)
        .with_context(|| format!("reading BSP file: {}", bsp_path.display()))?;

    let bsp = BspData::parse(BspParseInput {
        bsp: &bsp_bytes,
        lit: None,
        settings: BspParseSettings::default(),
    })
    .with_context(|| format!("parsing BSP: {}", bsp_path.display()))?;

    // ── 2. Extract entity waypoints ──────────────────────────────────────────
    let entity_lump_bytes = &bsp.entities;
    // Convert from Quake-string bytes to a lossy UTF-8 string.
    // We make a mutable copy so quake_string_to_utf8_lossy can strip null terminator.
    let mut entity_bytes_copy = entity_lump_bytes.clone();
    let entity_str = qbsp::util::quake_string_to_utf8_lossy(&mut entity_bytes_copy).to_owned();

    let entity_origins = crate::bsp::entity::extract_waypoints(&entity_str)
        .unwrap_or_else(|e| {
            eprintln!("cs-flythrough: warning: waypoint extraction failed: {e:#}");
            Vec::new()
        });

    // Convert from qbsp's glam::Vec3 (v0.30) to our glam::Vec3 (v0.32).
    // Both are identical repr ([f32; 3]) so we convert element-by-element.
    let entity_origins: Vec<Vec3> = entity_origins
        .into_iter()
        .map(|v| Vec3::new(v.x, v.y, v.z))
        .collect();

    // ── 3. Resolve WAD paths ─────────────────────────────────────────────────
    let wad_paths: Vec<std::path::PathBuf> = if let Some(wad_value) = find_worldspawn_wad(&entity_str) {
        resolve_wad_paths(wad_value, cs_install_path)
    } else {
        Vec::new()
    };

    // Also scan cstrike/ and valve/ for any WADs not listed in worldspawn.
    // This catches halflife.wad, decals.wad, and other shared texture sources.
    let wad_paths = {
        let mut paths = wad_paths;
        let scan_dirs = [
            cs_install_path.join("cstrike"),
            cs_install_path.join("valve"),
        ];
        for dir in &scan_dirs {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.extension().and_then(|e| e.to_str())
                        .map(|e| e.eq_ignore_ascii_case("wad"))
                        .unwrap_or(false)
                        && !paths.contains(&p)
                    {
                        paths.push(p);
                    }
                }
            }
        }
        paths
    };

    // ── 4. Collect texture names from BSP texture lump ───────────────────────
    // Filter out special-purpose GoldSrc textures that should never be rendered.
    // Loading them into the atlas causes their face pixels to show (aaatrigger is
    // literally a bright pink texture in the WADs). Excluding them here ensures the
    // atlas UV lookup returns None for those faces, and the face loop skips them.
    const NODRAW_NAMES: &[&str] = &[
        "aaatrigger", "trigger", "clip", "null",
        "hint", "skip", "nodraw",
    ];
    let texture_names: Vec<String> = bsp
        .textures
        .iter()
        .flatten()
        .map(|t| t.header.name.as_str().to_lowercase())
        .filter(|n| !NODRAW_NAMES.contains(&n.as_str()))
        .collect();

    // ── 5. Build diffuse atlas ───────────────────────────────────────────────
    let diffuse_atlas = load_textures(&texture_names, &wad_paths)
        .context("building diffuse texture atlas")?;

    // ── 6. Compute lightmap atlas ────────────────────────────────────────────
    let lm_settings = ComputeLightmapSettings {
        default_color: [0; 3],
        no_lighting_color: [0; 3],
        special_lighting_color: [255; 3],
        max_width: 2048,
        max_height: u32::MAX,
        extrusion: 1,
    };

    // Use PerStyleLightmapPacker (simpler output — one image per style).
    let lm_result = bsp
        .compute_lightmap_atlas(PerStyleLightmapPacker::new(lm_settings))
        .ok();

    let (lightmap_atlas, lightmap_uv_map): (RgbaImage, LightmapUvMap) = match lm_result {
        Some(output) => {
            let atlas = style_data_to_rgba(&output.data);
            (atlas, output.uvs)
        }
        None => {
            // No lighting data — use a 1×1 white pixel as fallback.
            let mut img = RgbaImage::new(1, 1);
            img.put_pixel(0, 0, image::Rgba([255, 255, 255, 255]));
            (img, LightmapUvMap::new())
        }
    };

    let lm_atlas_w = lightmap_atlas.width() as f32;
    let lm_atlas_h = lightmap_atlas.height() as f32;

    // ── 7. Build vertex + index buffers ──────────────────────────────────────
    //
    // We process model 0 (worldspawn) — all world geometry.
    // Faces are partitioned into geometry and sky.
    // Final layout:
    //   vertices = [geo_verts..., sky_verts...]
    //   indices  = [geo_indices..., sky_indices (adjusted)...]
    //   sky_index_offset = geo_indices.len()

    let mut geo_vertices: Vec<Vertex> = Vec::new();
    let mut geo_indices: Vec<u32> = Vec::new();
    let mut sky_vertices: Vec<Vertex> = Vec::new();
    let mut sky_indices: Vec<u32> = Vec::new();

    let worldspawn = &bsp.models[0];

    for face_idx in worldspawn.first_face..worldspawn.first_face + worldspawn.num_faces {
        let face = &bsp.faces[face_idx as usize];
        let tex_info = &bsp.tex_info[face.texture_info_idx.0 as usize];

        let sky = is_sky_face(&bsp, face);

        // Get texture name for atlas UV lookup.
        let tex_name = bsp
            .get_texture_name(tex_info)
            .map(|n| n.as_str().to_lowercase())
            .unwrap_or_default();

        // Skip invisible / special-purpose GoldSrc textures. These are real textures
        // present in the WADs (so they DO land in the atlas), but they exist only as
        // BSP-space markers: trigger volumes, player-clip planes, BSP hint/skip faces.
        // Rendering them produces garish magenta or electric-blue rectangles floating
        // in the playable space — skip them entirely.
        const NODRAW: &[&str] = &[
            "aaatrigger", "trigger", "clip", "null",
            "hint", "skip", "nodraw",
        ];
        if NODRAW.iter().any(|&n| tex_name == n) {
            continue;
        }

        // Diffuse UV rect from atlas. If the texture is not in the atlas (trigger
        // volumes, clip brushes, hint/skip faces — textures that exist only as BSP
        // names but have no pixels in any WAD), skip the face entirely rather than
        // rendering it with the magenta fallback. This eliminates the bright pink
        // rectangles that appear for func_bombsite zones, clip planes, etc.
        let uv_rect = match diffuse_atlas.uvs.get(&tex_name).copied() {
            Some(r) => r,
            None => continue,
        };

        // Texture dimensions for UV normalization.
        // qbsp's projection.project() returns world-space (texture-space) UVs,
        // not normalized 0..1. We must divide by texture dimensions.
        let (tex_w, tex_h) = bsp
            .textures
            .iter()
            .flatten()
            .find(|t| t.header.name.as_str().to_lowercase() == tex_name)
            .map(|t| (t.header.width as f32, t.header.height as f32))
            .unwrap_or((64.0, 64.0));

        // Gather per-vertex lightmap UVs for this face (already normalized 0..1 by qbsp).
        let lm_uvs: Vec<[f32; 2]> = if let Some(face_lm_uvs) = lightmap_uv_map.get(&face_idx) {
            face_lm_uvs.iter().map(|uv| [uv.x, uv.y]).collect()
        } else {
            // No lightmap for this face; point all verts at centre of atlas (white pixel).
            vec![[0.5 / lm_atlas_w, 0.5 / lm_atlas_h]; face.num_edges.0 as usize]
        };

        // Collect face vertices.
        let face_verts: Vec<qbsp::glam::Vec3> = face.vertices(&bsp).collect();
        let n = face_verts.len();
        if n < 3 {
            continue;
        }

        let target_verts = if sky { &mut sky_vertices } else { &mut geo_vertices };
        let target_idxs = if sky { &mut sky_indices } else { &mut geo_indices };

        let base_vertex = target_verts.len() as u32;

        for (vi, qbsp_pos) in face_verts.iter().enumerate() {
            // Project vertex onto the texture plane to get texture-space UVs.
            let raw_uv = tex_info.projection.project(*qbsp_pos);
            // Normalize from texture-space to 0..1, then wrap.
            let u_norm = (raw_uv.x / tex_w).rem_euclid(1.0);
            let v_norm = (raw_uv.y / tex_h).rem_euclid(1.0);
            // Remap into the atlas UV rect.
            let u_atlas = uv_rect[0] + u_norm * (uv_rect[2] - uv_rect[0]);
            let v_atlas = uv_rect[1] + v_norm * (uv_rect[3] - uv_rect[1]);

            // Lightmap UV (already atlas-normalized from qbsp).
            let lm_uv = lm_uvs.get(vi).copied().unwrap_or([0.0, 0.0]);

            // Convert position from qbsp's glam 0.30 Vec3 to [f32; 3].
            target_verts.push(Vertex {
                position: [qbsp_pos.x, qbsp_pos.y, qbsp_pos.z],
                diffuse_uv: [u_atlas, v_atlas],
                lightmap_uv: lm_uv,
            });
        }

        // Fan triangulation: vertices 0, 1, 2 / 0, 2, 3 / 0, 3, 4 …
        for i in 1..n as u32 - 1 {
            target_idxs.push(base_vertex);
            target_idxs.push(base_vertex + i + 1);
            target_idxs.push(base_vertex + i);
        }
    }

    // Merge: geo first, then sky (sky vertex indices offset by geo_vertices.len()).
    let sky_index_offset = geo_indices.len() as u32;
    let geo_vertex_count = geo_vertices.len() as u32;

    let mut vertices = geo_vertices;
    vertices.extend(sky_vertices);

    let mut indices = geo_indices;
    for idx in sky_indices {
        indices.push(idx + geo_vertex_count);
    }

    Ok(MeshData {
        vertices,
        indices,
        sky_index_offset,
        diffuse_atlas: diffuse_atlas.image,
        lightmap_atlas,
        entity_origins,
    })
}
