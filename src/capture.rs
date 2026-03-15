use std::path::Path;
use anyhow::{Context, Result};
use glam::Vec3;

pub struct FrameEntry {
    pub frame: u32,
    pub pos: [f32; 3],
    pub angle: [f32; 2], // [yaw_deg, pitch_deg]
    pub file: String,
}

/// Strip wgpu 256-byte row-alignment padding from raw texture readback data.
/// Returns tightly-packed RGBA bytes (width * 4 * height).
pub fn strip_padding(padded: &[u8], width: u32, height: u32, padded_bpr: u32) -> Vec<u8> {
    let unpadded_bpr = (width * 4) as usize;
    let padded_bpr = padded_bpr as usize;
    let mut out = vec![0u8; unpadded_bpr * height as usize];
    for row in 0..height as usize {
        let src_start = row * padded_bpr;
        let dst_start = row * unpadded_bpr;
        out[dst_start..dst_start + unpadded_bpr]
            .copy_from_slice(&padded[src_start..src_start + unpadded_bpr]);
    }
    out
}

/// Write a PNG from tightly-packed RGBA pixel bytes (already stripped of padding).
pub fn write_png(path: &Path, pixels: &[u8], width: u32, height: u32) -> Result<()> {
    let img = image::RgbaImage::from_raw(width, height, pixels.to_vec())
        .ok_or_else(|| anyhow::anyhow!("pixel buffer size mismatch for {}x{}", width, height))?;
    img.save(path).with_context(|| format!("writing PNG to {}", path.display()))
}

/// Print a single-frame JSON line to stdout.
pub fn print_frame_json(frame: u32, eye: Vec3, yaw_deg: f32, pitch_deg: f32, file: &Path) {
    let file_escaped = file.display().to_string().replace('\\', "\\\\").replace('"', "\\\"");
    println!(
        "{{\"frame\": {frame}, \"pos\": [{:.3}, {:.3}, {:.3}], \"angle\": [{:.3}, {:.3}], \"file\": \"{file_escaped}\"}}",
        eye.x, eye.y, eye.z, yaw_deg, pitch_deg,
    );
}

/// Print a fatal error JSON line to stdout.
pub fn print_error_json(msg: &str) {
    let escaped = msg.replace('\\', "\\\\").replace('"', "\\\"");
    println!("{{\"error\": \"{escaped}\"}}");
}

/// Write walkthrough manifest JSON to path.
pub fn write_manifest(
    path: &Path,
    map: &str,
    width: u32,
    height: u32,
    frame_step: u32,
    frames: &[FrameEntry],
) -> Result<()> {
    let map_escaped = map.replace('\\', "\\\\").replace('"', "\\\"");
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str(&format!("  \"map\": \"{map_escaped}\",\n"));
    s.push_str(&format!("  \"resolution\": \"{width}x{height}\",\n"));
    s.push_str(&format!("  \"frame_step\": {frame_step},\n"));
    s.push_str("  \"frames\": [\n");
    for (i, f) in frames.iter().enumerate() {
        let comma = if i + 1 < frames.len() { "," } else { "" };
        let file_escaped = f.file.replace('\\', "\\\\").replace('"', "\\\"");
        s.push_str(&format!(
            "    {{\"frame\": {}, \"pos\": [{:.3}, {:.3}, {:.3}], \"angle\": [{:.3}, {:.3}], \"file\": \"{}\"}}{}\n",
            f.frame, f.pos[0], f.pos[1], f.pos[2], f.angle[0], f.angle[1], file_escaped, comma
        ));
    }
    s.push_str("  ]\n");
    s.push('}');
    std::fs::write(path, s).with_context(|| format!("writing manifest to {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_padding_removes_alignment_bytes() {
        // 2×2 image: unpadded_bpr = 8, padded to 256 (wgpu minimum alignment)
        let width = 2u32;
        let height = 2u32;
        let padded_bpr = 256usize;
        let mut input = vec![0u8; padded_bpr * height as usize];
        input[..8].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        input[256..264].copy_from_slice(&[9, 10, 11, 12, 13, 14, 15, 16]);
        let result = strip_padding(&input, width, height, padded_bpr as u32);
        assert_eq!(result, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
    }

    #[test]
    fn test_strip_padding_no_padding_needed() {
        // 64×1 image: unpadded_bpr = 256, which happens to equal padded_bpr
        let width = 64u32;
        let height = 1u32;
        let padded_bpr = 256u32;
        let input: Vec<u8> = (0u8..=255u8).collect();
        let result = strip_padding(&input, width, height, padded_bpr);
        assert_eq!(result, input);
    }

    #[test]
    fn test_write_png_roundtrip() {
        // 2×2 solid red RGBA image — write to temp file, read back, verify byte fidelity.
        // Note: write_png treats bytes as opaque RGBA data. sRGB encoding is applied by
        // the GPU before bytes reach this function — this test checks byte fidelity only.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.png");
        let pixels: Vec<u8> = vec![255, 0, 0, 255].repeat(4); // 4 red pixels
        write_png(&path, &pixels, 2, 2).unwrap();
        let img = image::open(&path).unwrap().to_rgba8();
        assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn test_write_manifest_structure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("walkthrough.json");
        let frames = vec![
            FrameEntry { frame: 0, pos: [1.0, 2.0, 3.0], angle: [90.0, 0.0], file: "frame_0000.png".into() },
            FrameEntry { frame: 1, pos: [4.0, 5.0, 6.0], angle: [91.0, 1.0], file: "frame_0001.png".into() },
        ];
        write_manifest(&path, "de_dust2", 1920, 1080, 60, &frames).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.trim_start().starts_with('{'), "manifest must start with {{");
        assert!(content.trim_end().ends_with('}'), "manifest must end with }}");
        let opens = content.chars().filter(|&c| c == '{').count();
        let closes = content.chars().filter(|&c| c == '}').count();
        assert_eq!(opens, closes, "unbalanced braces in manifest JSON");
        assert!(content.contains("\"map\": \"de_dust2\""));
        assert!(content.contains("\"resolution\": \"1920x1080\""));
        assert!(content.contains("\"frame_step\": 60"));
        assert!(content.contains("\"frame\": 0"));
        assert!(content.contains("\"frame\": 1"));
        assert!(content.contains("frame_0000.png"));
        assert!(content.contains("frame_0001.png"));
        let last_frame_pos = content.rfind("frame_0001.png").unwrap();
        let after_last = &content[last_frame_pos..];
        let closing_brace = after_last.find('}').unwrap();
        let between = &after_last[closing_brace + 1..];
        assert!(!between.trim_start().starts_with(','), "trailing comma after last frame");
    }

    #[test]
    fn test_write_manifest_file_field_escaped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.json");
        let frames = vec![
            FrameEntry { frame: 0, pos: [0.0, 0.0, 0.0], angle: [0.0, 0.0], file: r"sub\frame_0000.png".into() },
        ];
        write_manifest(&path, "de_dust2", 1920, 1080, 60, &frames).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains(r"sub\\frame_0000.png"), "backslash in file path must be escaped");
    }
}
