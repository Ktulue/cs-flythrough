use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use guillotiere::{size2, AtlasAllocator};
use image::RgbaImage;

use goldsrc_rs::{
    common::cstring_bytes,
    texture::mip_texture,
    wad::{wad, wad_entry},
};

const ATLAS_SIZE: i32 = 4096;

/// Packed texture atlas with UV coordinates for each texture.
pub struct TextureAtlas {
    /// RGBA image containing all packed textures.
    pub image: RgbaImage,
    /// Maps texture name to normalized UV rect [u_min, v_min, u_max, v_max].
    pub uvs: HashMap<String, [f32; 4]>,
}

/// Convert palette-indexed texture data to RGBA pixels.
///
/// GoldSrc convention: palette index 255 is the transparency color (alpha = 0).
fn indexed_to_rgba(indices: &[u8], palette: &[[u8; 3]], width: u32, height: u32) -> Vec<u8> {
    let pixel_count = (width * height) as usize;
    let mut rgba = Vec::with_capacity(pixel_count * 4);
    for &idx in indices.iter().take(pixel_count) {
        if idx == 255 {
            // Transparency color — fully transparent.
            rgba.extend_from_slice(&[0u8, 0u8, 0u8, 0u8]);
        } else {
            let [r, g, b] = palette.get(idx as usize).copied().unwrap_or([0u8, 0u8, 0u8]);
            rgba.extend_from_slice(&[r, g, b, 255u8]);
        }
    }
    rgba
}

/// Build a map of lowercase texture name → RGBA pixel data + dimensions from a single WAD file.
fn load_wad_textures(
    path: &Path,
) -> Result<HashMap<String, (Vec<u8>, u32, u32)>> {
    let data = std::fs::read(path)
        .with_context(|| format!("failed to read WAD file: {}", path.display()))?;

    let wad_file = wad(&data)
        .with_context(|| format!("failed to parse WAD file: {}", path.display()))?;

    let mut map: HashMap<String, (Vec<u8>, u32, u32)> = HashMap::new();

    for entry in wad_file.entries.iter() {
        // Only process texture lumps (type 0x43 = miptex).
        if entry.ty != 0x43 {
            continue;
        }
        if entry.compression != 0 {
            // Skip compressed lumps — unsupported.
            continue;
        }

        let raw_name = cstring_bytes(&entry.name);
        let name = String::from_utf8_lossy(raw_name).to_lowercase();
        if name.is_empty() {
            continue;
        }

        let bytes = match wad_entry(&data, entry) {
            Ok(b) => b,
            Err(_) => continue,
        };

        let miptex = match mip_texture(bytes) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let Some(color_data) = miptex.data else {
            continue;
        };

        let width = miptex.header.width.get();
        let height = miptex.header.height.get();
        if width == 0 || height == 0 {
            continue;
        }

        // Use mip level 0 (full resolution).
        let rgba = indexed_to_rgba(color_data.indices[0], color_data.palette, width, height);
        map.insert(name, (rgba, width, height));
    }

    Ok(map)
}

/// Load the requested textures from any of the provided WAD files and pack them into
/// a single RGBA atlas using guillotiere bin-packing.
///
/// Textures not found in any WAD get a 16×16 magenta fallback so rendering can
/// continue for textures that are embedded in the BSP itself.
pub fn load_textures(
    texture_names: &[String],
    wad_paths: &[impl AsRef<Path>],
) -> Result<TextureAtlas> {
    // Build a combined map from all WAD files. Later WADs win on name collision.
    let mut all_textures: HashMap<String, (Vec<u8>, u32, u32)> = HashMap::new();
    for path in wad_paths {
        match load_wad_textures(path.as_ref()) {
            Ok(map) => {
                all_textures.extend(map);
            }
            Err(e) => {
                // Non-fatal: log and continue so missing WADs don't abort loading.
                eprintln!("cs-flythrough: warning: {e:#}");
            }
        }
    }


    // Deduplicate names while preserving order.
    let unique_names: Vec<&String> = {
        let mut seen = std::collections::HashSet::new();
        texture_names
            .iter()
            .filter(|n| seen.insert(n.to_lowercase()))
            .collect()
    };

    // Allocate atlas space using guillotiere.
    let mut allocator = AtlasAllocator::new(size2(ATLAS_SIZE, ATLAS_SIZE));
    let mut atlas_image = RgbaImage::new(ATLAS_SIZE as u32, ATLAS_SIZE as u32);
    let mut uvs: HashMap<String, [f32; 4]> = HashMap::new();

    for name in unique_names {
        let key = name.to_lowercase();
        let Some((pixels, width, height)) = all_textures
            .get(&key)
            .map(|(p, w, h)| (p.as_slice(), *w, *h))
        else {
            // Texture not found in any WAD — skip it entirely. The face-loop in
            // parse.rs will hit `None => continue` for these faces.
            crate::diag!("[cs-flythrough] texture not in WAD, skipping: '{name}'");
            continue;
        };

        let alloc = match allocator.allocate(size2(width as i32, height as i32)) {
            Some(a) => a,
            None => {
                eprintln!(
                    "cs-flythrough: warning: atlas full, skipping texture '{name}'"
                );
                continue;
            }
        };

        let x0 = alloc.rectangle.min.x as u32;
        let y0 = alloc.rectangle.min.y as u32;

        // Blit texture pixels into the atlas image.
        for row in 0..height {
            for col in 0..width {
                let src_idx = ((row * width + col) as usize) * 4;
                let pixel = image::Rgba([
                    pixels[src_idx],
                    pixels[src_idx + 1],
                    pixels[src_idx + 2],
                    pixels[src_idx + 3],
                ]);
                atlas_image.put_pixel(x0 + col, y0 + row, pixel);
            }
        }

        let atlas_f = ATLAS_SIZE as f32;
        let uv = [
            x0 as f32 / atlas_f,
            y0 as f32 / atlas_f,
            (x0 + width) as f32 / atlas_f,
            (y0 + height) as f32 / atlas_f,
        ];
        uvs.insert(name.clone(), uv);
    }

    Ok(TextureAtlas {
        image: atlas_image,
        uvs,
    })
}
