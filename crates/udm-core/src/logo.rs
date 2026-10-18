//! The UDM logo, drawn in code: a blue rounded square with a white download
//! arrow made of three parallel "connection" bars over a base line.
//!
//! Rendering is a pure function of the edge length so every front-end (window,
//! tray, `.exe` resource, browser extension) gets the same pixels and no binary
//! image asset is checked in.

use crate::limits::MAX_ICON_PX;

/// Supersampling grid per axis (SS × SS samples per pixel).
const SS: u8 = 4;
/// Samples per pixel.
const SAMPLES: u32 = 16;
/// Gradient colours (top, bottom) of the background.
const TOP: [u8; 3] = [0x3B, 0x8B, 0xF5];
const BOTTOM: [u8; 3] = [0x1C, 0x4F, 0xBF];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hit {
    Outside,
    Background,
    Glyph,
}

/// Signed-distance-free inside test for a rounded rectangle.
fn in_rounded_rect(x: f32, y: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> bool {
    if x < x0 || x > x1 || y < y0 || y > y1 {
        return false;
    }
    // `max`/`min` instead of `clamp`: `clamp` panics when the bounds cross by rounding.
    let cx = x.max(x0 + r).min(x1 - r);
    let cy = y.max(y0 + r).min(y1 - r);
    let (dx, dy) = (x - cx, y - cy);
    dx * dx + dy * dy <= r * r
}

/// Point inside the triangle (a, b, c), any winding.
fn in_triangle(p: (f32, f32), a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let side = |u: (f32, f32), v: (f32, f32)| (v.0 - u.0) * (p.1 - u.1) - (v.1 - u.1) * (p.0 - u.0);
    let (d1, d2, d3) = (side(a, b), side(b, c), side(c, a));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

/// Classify a point in normalised coordinates (0..1, y down).
fn hit(x: f32, y: f32) -> Hit {
    if !in_rounded_rect(x, y, 0.03, 0.03, 0.97, 0.97, 0.21) {
        return Hit::Outside;
    }
    let bars = [0.36_f32, 0.50, 0.64]
        .iter()
        .any(|&bx| in_rounded_rect(x, y, bx - 0.045, 0.17, bx + 0.045, 0.53, 0.045));
    let head = in_triangle((x, y), (0.25, 0.47), (0.75, 0.47), (0.5, 0.73));
    let base = in_rounded_rect(x, y, 0.22, 0.79, 0.78, 0.86, 0.035);
    if bars || head || base {
        Hit::Glyph
    } else {
        Hit::Background
    }
}

/// `(a * (n - k) + b * k) / n` on bytes.
fn lerp(a: u8, b: u8, k: u32, n: u32) -> Option<u8> {
    let rest = n.checked_sub(k)?;
    let v = u32::from(a)
        .checked_mul(rest)?
        .checked_add(u32::from(b).checked_mul(k)?)?
        .checked_div(n)?;
    u8::try_from(v).ok()
}

/// Straight-alpha RGBA pixels, row-major from the top, `size × size`.
/// `None` for `size == 0` or `size > MAX_ICON_PX`.
pub fn rgba(size: u16) -> Option<Vec<u8>> {
    if size == 0 || size > MAX_ICON_PX {
        return None;
    }
    let n = usize::from(size);
    let mut out = Vec::with_capacity(n.checked_mul(n)?.checked_mul(4)?);
    let edge = f32::from(size);
    let rows = u32::from(size).saturating_sub(1).max(1);
    for py in 0..size {
        let grad = [
            lerp(TOP[0], BOTTOM[0], u32::from(py), rows)?,
            lerp(TOP[1], BOTTOM[1], u32::from(py), rows)?,
            lerp(TOP[2], BOTTOM[2], u32::from(py), rows)?,
        ];
        for px in 0..size {
            let (mut inside, mut glyph) = (0_u32, 0_u32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (f32::from(px) + (f32::from(sx) + 0.5) / f32::from(SS)) / edge;
                    let y = (f32::from(py) + (f32::from(sy) + 0.5) / f32::from(SS)) / edge;
                    match hit(x, y) {
                        Hit::Outside => {}
                        Hit::Background => inside = inside.saturating_add(1),
                        Hit::Glyph => {
                            inside = inside.saturating_add(1);
                            glyph = glyph.saturating_add(1);
                        }
                    }
                }
            }
            if inside == 0 {
                out.extend_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            for c in grad {
                out.push(lerp(c, 0xFF, glyph, inside)?);
            }
            out.push(lerp(0, 0xFF, inside, SAMPLES)?);
        }
    }
    Some(out)
}

/// An ICO/`RT_ICON` image: `BITMAPINFOHEADER` + bottom-up 32-bit BGRA + AND mask.
pub fn dib(size: u16) -> Option<Vec<u8>> {
    let px = rgba(size)?;
    let n = usize::from(size);
    let row = n.checked_mul(4)?;
    // AND mask rows are padded to 32 bits; all zero because alpha is used.
    let mask_row = n.checked_add(31)?.checked_div(32)?.checked_mul(4)?;
    let pixels_len = row.checked_mul(n)?;
    let mask_len = mask_row.checked_mul(n)?;
    let image_len = u32::try_from(pixels_len.checked_add(mask_len)?).ok()?;
    let width = i32::from(size);
    let height = width.checked_mul(2)?;
    let mut out = Vec::with_capacity(40_usize.checked_add(pixels_len)?.checked_add(mask_len)?);
    out.extend_from_slice(&40_u32.to_le_bytes());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&32_u16.to_le_bytes());
    out.extend_from_slice(&0_u32.to_le_bytes());
    out.extend_from_slice(&image_len.to_le_bytes());
    out.extend_from_slice(&[0; 16]);
    for line in px.chunks_exact(row).rev() {
        for [r, g, b, a] in line.as_chunks::<4>().0 {
            out.extend_from_slice(&[*b, *g, *r, *a]);
        }
    }
    out.resize(out.len().checked_add(mask_len)?, 0);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(px: &[u8], size: u16, x: u16, y: u16) -> [u8; 4] {
        let i = (usize::from(y) * usize::from(size) + usize::from(x)) * 4;
        [px[i], px[i + 1], px[i + 2], px[i + 3]]
    }

    #[test]
    fn renders_corners_transparent_and_glyph_white() {
        let s = 64;
        let px = rgba(s).unwrap();
        assert_eq!(px.len(), 64 * 64 * 4);
        assert_eq!(at(&px, s, 0, 0)[3], 0);
        // Middle bar and arrow tip area are white and opaque.
        assert_eq!(at(&px, s, 32, 20), [255, 255, 255, 255]);
        assert_eq!(at(&px, s, 32, 40), [255, 255, 255, 255]);
        // Background between bars is blue.
        let bg = at(&px, s, 12, 32);
        assert_eq!(bg[3], 255);
        assert!(bg[2] > bg[0]);
    }

    #[test]
    fn dib_layout() {
        let d = dib(16).unwrap();
        assert_eq!(d.len(), 40 + 16 * 16 * 4 + 4 * 16);
        assert_eq!(&d[4..8], &16_i32.to_le_bytes());
        assert_eq!(&d[8..12], &32_i32.to_le_bytes());
    }

    #[test]
    fn hostile_icon_size_is_bounded() {
        assert!(rgba(0).is_none());
        assert!(rgba(MAX_ICON_PX + 1).is_none());
        assert!(dib(u16::MAX).is_none());
        assert_eq!(rgba(MAX_ICON_PX).unwrap().len(), 256 * 256 * 4);
    }
}
