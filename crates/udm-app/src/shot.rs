//! `ui.screenshot`: path validation and PNG encoding.

use std::path::{Component, Path, PathBuf};

use eframe::egui::ColorImage;
use udm_core::{Message, limits};

/// Validates a screenshot target: a `.png` path without `..` components.
pub fn check_path(raw: &str) -> Result<PathBuf, Message> {
    let bad = || Message::new("error-ui-screenshot-path");
    if raw.is_empty() || raw.len() > limits::MAX_PATH_BYTES || raw.contains('\0') {
        return Err(bad());
    }
    let p = PathBuf::from(raw);
    let is_png = p
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("png"));
    if !is_png || p.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(bad());
    }
    Ok(p)
}

/// Encodes RGBA pixels as PNG bytes.
pub fn encode_png(img: &ColorImage) -> Result<Vec<u8>, Message> {
    let fail = || Message::new("error-ui-screenshot-write");
    let [w, h] = img.size;
    let w = u32::try_from(w).map_err(|_| fail())?;
    let h = u32::try_from(h).map_err(|_| fail())?;
    let mut rgba = Vec::with_capacity(img.pixels.len().saturating_mul(4));
    for c in &img.pixels {
        rgba.extend_from_slice(&c.to_srgba_unmultiplied());
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|_| fail())?;
        writer.write_image_data(&rgba).map_err(|_| fail())?;
        writer.finish().map_err(|_| fail())?;
    }
    Ok(out)
}

/// Encodes and writes the image, creating parent folders.
pub fn save(path: &Path, img: &ColorImage) -> Result<u64, Message> {
    let bytes = encode_png(img)?;
    let fail = |_| Message::new("error-ui-screenshot-write");
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(fail)?;
    }
    std::fs::write(path, &bytes).map_err(fail)?;
    Ok(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Color32;

    #[test]
    fn hostile_screenshot_paths() {
        for bad in ["", "a.txt", "../x.png", "a/../../x.png", "x.png\0", "noext"] {
            assert!(check_path(bad).is_err(), "{bad:?}");
        }
        assert!(check_path(&"a".repeat(limits::MAX_PATH_BYTES + 1)).is_err());
        assert!(check_path("target/shots/main-en.png").is_ok());
    }

    #[test]
    fn writes_png() {
        let dir = tempfile::tempdir().unwrap();
        let img = ColorImage::new([3, 2], vec![Color32::RED; 6]);
        let p = dir.path().join("s/x.png");
        assert!(save(&p, &img).unwrap() > 0);
        let data = std::fs::read(&p).unwrap();
        assert_eq!(&data[1..4], b"PNG");
    }
}
