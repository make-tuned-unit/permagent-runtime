//! Local still for a social_post.
//!
//! Tries a user-run mflux HTTP sidecar (loopback, configurable URL). If it is
//! down, composes a branded PNG from *this project's* kit. Fallback palette is
//! a neutral poster, not any product's identity — used only when the project
//! has not saved a brand yet.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::Config;
use crate::projects::ProjectBrand;

pub const MFLUX_URL_KEY: &str = "mflux_url";
const DEFAULT_MFLUX: &str = "http://127.0.0.1:7861";
const MFLUX_PROBE: Duration = Duration::from_secs(2);
const MFLUX_CALL: Duration = Duration::from_secs(120);

pub struct StillSpec {
    pub title: String,
    pub body: String,
    pub project_name: String,
    pub brand: ProjectBrand,
    pub format: String,
    /// Taste notes for this regeneration. Never lettered onto the graphic.
    pub feedback: String,
}

pub fn mflux_url() -> String {
    Config::global()
        .get_param::<String>(MFLUX_URL_KEY)
        .ok()
        .map(|s| s.trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_MFLUX.to_string())
}

pub fn prompt_for(spec: &StillSpec) -> String {
    let mut parts = vec![spec.title.trim().to_string()];
    if !spec.body.trim().is_empty() {
        parts.push(spec.body.trim().chars().take(280).collect());
    }
    if !spec.brand.voice.is_empty() {
        parts.push(format!("Voice: {}", spec.brand.voice));
    }
    if !spec.brand.donts.is_empty() {
        parts.push(format!("Do not: {}", spec.brand.donts.join("; ")));
    }
    if !spec.brand.bg.is_empty() {
        parts.push(format!(
            "Palette bg {} fg {} accent {}",
            spec.brand.bg, spec.brand.fg, spec.brand.accent
        ));
    }
    if !spec.feedback.trim().is_empty() {
        parts.push(format!(
            "Revise from prior still using this direction (do not letter these notes onto the image): {}",
            spec.feedback.trim().chars().take(400).collect::<String>()
        ));
    }
    parts.push(format!(
        "Square-to-portrait graphic for {}, no fake product UI, no watermark.",
        spec.project_name
    ));
    parts.join("\n")
}

pub fn dimensions_for(format: &str) -> (u32, u32) {
    match format {
        "reel" => (1080, 1920),
        "li" | "linkedin" => (1200, 627),
        _ => (1080, 1350),
    }
}

/// Neutral poster used only when this project has no kit yet. Intentionally
/// not a recognizable product palette.
pub fn fallback_palette() -> (u8, u8, u8, u8, u8, u8, u8, u8, u8) {
    (17, 17, 17, 245, 245, 240, 200, 190, 170)
}

pub fn parse_hex(color: &str) -> Option<(u8, u8, u8)> {
    let b = color.as_bytes();
    if b.len() != 7 || b[0] != b'#' {
        return None;
    }
    let n = u32::from_str_radix(std::str::from_utf8(&b[1..]).ok()?, 16).ok()?;
    Some(((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

fn palette(brand: &ProjectBrand) -> (u8, u8, u8, u8, u8, u8, u8, u8, u8) {
    let (dbg, dfg, dacc) = (
        fallback_palette().0,
        fallback_palette().3,
        fallback_palette().6,
    );
    let (bg_r, bg_g, bg_b) =
        parse_hex(&brand.bg).unwrap_or((dbg, fallback_palette().1, fallback_palette().2));
    let (fg_r, fg_g, fg_b) =
        parse_hex(&brand.fg).unwrap_or((dfg, fallback_palette().4, fallback_palette().5));
    let (ac_r, ac_g, ac_b) =
        parse_hex(&brand.accent).unwrap_or((dacc, fallback_palette().7, fallback_palette().8));
    (bg_r, bg_g, bg_b, fg_r, fg_g, fg_b, ac_r, ac_g, ac_b)
}

fn layout_seed(feedback: &str) -> u32 {
    let mut h = 2166136261u32;
    for b in feedback.as_bytes() {
        h ^= u32::from(*b);
        h = h.wrapping_mul(16777619);
    }
    h
}

pub fn compose_still(spec: &StillSpec, dir: &Path) -> Result<PathBuf, String> {
    let (w, h) = dimensions_for(&spec.format);
    let (br, bg, bb, fr, fg, fb, ar, ag, ab) = palette(&spec.brand);
    let mut img = image::RgbImage::from_pixel(w, h, image::Rgb([br, bg, bb]));
    let seed = layout_seed(&spec.feedback);
    let bar = (h / 48).max(16) + (seed % 12);

    // Accent bar at the top — structural, not a logo. Thickness shifts a
    // little when taste notes change so a compose retry is not a duplicate.
    for y in 0..bar {
        for x in 0..w {
            img.put_pixel(x, y, image::Rgb([ar, ag, ab]));
        }
    }

    let margin = w / 12;
    let mut y = h / 8 + 24 + (seed % 40);
    y = draw_text(
        &mut img,
        spec.project_name.trim(),
        margin,
        y,
        3,
        fr,
        fg,
        fb,
        w - margin,
    );
    y += 28;
    draw_text(
        &mut img,
        spec.title.trim(),
        margin,
        y,
        6,
        fr,
        fg,
        fb,
        w - margin,
    );

    let png = dir.join("still.png");
    img.save(&png).map_err(|e| e.to_string())?;

    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">
  <rect width="100%" height="100%" fill="#{:02x}{:02x}{:02x}"/>
  <rect width="100%" height="{bar}" fill="#{:02x}{:02x}{:02x}"/>
  <text x="{margin}" y="{}" fill="#{:02x}{:02x}{:02x}" font-size="28" font-family="system-ui,sans-serif">{}</text>
  <text x="{margin}" y="{}" fill="#{:02x}{:02x}{:02x}" font-size="56" font-family="system-ui,sans-serif">{}</text>
</svg>"##,
        br,
        bg,
        bb,
        ar,
        ag,
        ab,
        h / 8,
        fr,
        fg,
        fb,
        xml_escape(spec.project_name.trim()),
        h / 8 + 80,
        fr,
        fg,
        fb,
        xml_escape(spec.title.trim()),
    );
    std::fs::write(dir.join("still.svg"), svg).map_err(|e| e.to_string())?;
    Ok(png)
}

fn xml_escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '&' => "&amp;".into(),
            '"' => "&quot;".into(),
            other => other.to_string(),
        })
        .collect()
}

/// 5×7 bitmap font for a short ASCII subset. Unknown glyphs become a box so
/// we never pull a system font (or a particular brand typeface) into the
/// daemon.
fn glyph(c: char) -> [u8; 5] {
    match c.to_ascii_uppercase() {
        'A' => [0x3E, 0x48, 0x48, 0x48, 0x3E],
        'B' => [0x7E, 0x4A, 0x4A, 0x4A, 0x34],
        'C' => [0x3C, 0x42, 0x42, 0x42, 0x24],
        'D' => [0x7E, 0x42, 0x42, 0x42, 0x3C],
        'E' => [0x7E, 0x4A, 0x4A, 0x4A, 0x42],
        'F' => [0x7E, 0x48, 0x48, 0x48, 0x40],
        'G' => [0x3C, 0x42, 0x4A, 0x4A, 0x2C],
        'H' => [0x7E, 0x08, 0x08, 0x08, 0x7E],
        'I' => [0x42, 0x42, 0x7E, 0x42, 0x42],
        'J' => [0x04, 0x02, 0x42, 0x7C, 0x40],
        'K' => [0x7E, 0x08, 0x14, 0x22, 0x42],
        'L' => [0x7E, 0x02, 0x02, 0x02, 0x02],
        'M' => [0x7E, 0x20, 0x18, 0x20, 0x7E],
        'N' => [0x7E, 0x20, 0x18, 0x04, 0x7E],
        'O' => [0x3C, 0x42, 0x42, 0x42, 0x3C],
        'P' => [0x7E, 0x48, 0x48, 0x48, 0x30],
        'Q' => [0x3C, 0x42, 0x46, 0x42, 0x3D],
        'R' => [0x7E, 0x48, 0x4C, 0x4A, 0x30],
        'S' => [0x32, 0x4A, 0x4A, 0x4A, 0x26],
        'T' => [0x40, 0x40, 0x7E, 0x40, 0x40],
        'U' => [0x7C, 0x02, 0x02, 0x02, 0x7C],
        'V' => [0x78, 0x04, 0x02, 0x04, 0x78],
        'W' => [0x7E, 0x04, 0x18, 0x04, 0x7E],
        'X' => [0x66, 0x18, 0x18, 0x18, 0x66],
        'Y' => [0x70, 0x08, 0x06, 0x08, 0x70],
        'Z' => [0x46, 0x4A, 0x52, 0x62, 0x42],
        '0' => [0x3C, 0x4A, 0x52, 0x62, 0x3C],
        '1' => [0x00, 0x22, 0x7E, 0x02, 0x00],
        '2' => [0x26, 0x4A, 0x4A, 0x4A, 0x32],
        '3' => [0x24, 0x42, 0x4A, 0x4A, 0x34],
        '4' => [0x78, 0x08, 0x08, 0x7E, 0x08],
        '5' => [0x7A, 0x4A, 0x4A, 0x4A, 0x44],
        '6' => [0x3C, 0x4A, 0x4A, 0x4A, 0x24],
        '7' => [0x40, 0x46, 0x48, 0x50, 0x60],
        '8' => [0x34, 0x4A, 0x4A, 0x4A, 0x34],
        '9' => [0x30, 0x4A, 0x4A, 0x4A, 0x3C],
        ' ' => [0x00, 0x00, 0x00, 0x00, 0x00],
        '.' => [0x00, 0x00, 0x02, 0x00, 0x00],
        ',' => [0x00, 0x00, 0x03, 0x00, 0x00],
        '!' => [0x00, 0x00, 0x7A, 0x00, 0x00],
        '?' => [0x20, 0x40, 0x4A, 0x48, 0x30],
        ':' => [0x00, 0x00, 0x24, 0x00, 0x00],
        '-' => [0x08, 0x08, 0x08, 0x08, 0x08],
        '\'' => [0x00, 0x00, 0x60, 0x00, 0x00],
        '"' => [0x00, 0x60, 0x00, 0x60, 0x00],
        '/' => [0x06, 0x08, 0x10, 0x20, 0x60],
        _ => [0x7E, 0x42, 0x42, 0x42, 0x7E],
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_text(
    img: &mut image::RgbImage,
    text: &str,
    x0: u32,
    y0: u32,
    scale: u32,
    r: u8,
    g: u8,
    b: u8,
    max_x: u32,
) -> u32 {
    let glyph_w = 6 * scale;
    let glyph_h = 8 * scale;
    let max_chars = ((max_x.saturating_sub(x0)) / glyph_w).max(1) as usize;
    let mut y = y0;
    for line in wrap(text, max_chars) {
        let mut x = x0;
        for ch in line.chars() {
            let bits = glyph(ch);
            for (col, column) in bits.iter().enumerate() {
                for row in 0..7u32 {
                    if column & (1 << row) != 0 {
                        for dy in 0..scale {
                            for dx in 0..scale {
                                let px = x + col as u32 * scale + dx;
                                let py = y + row * scale + dy;
                                if px < img.width() && py < img.height() {
                                    img.put_pixel(px, py, image::Rgb([r, g, b]));
                                }
                            }
                        }
                    }
                }
            }
            x += glyph_w;
            if x + glyph_w > max_x {
                break;
            }
        }
        y += glyph_h + scale;
        if y + glyph_h > img.height().saturating_sub(40) {
            break;
        }
    }
    y
}

fn wrap(text: &str, max_chars: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if cur.is_empty() {
            cur = word.to_string();
        } else if cur.len() + 1 + word.len() <= max_chars {
            cur.push(' ');
            cur.push_str(word);
        } else {
            lines.push(cur);
            cur = word.to_string();
        }
        if lines.len() >= 8 {
            break;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

pub async fn try_mflux_still(spec: &StillSpec, dir: &Path) -> Result<PathBuf, String> {
    let base = mflux_url();
    let probe = reqwest::Client::builder()
        .timeout(MFLUX_PROBE)
        .build()
        .map_err(|e| e.to_string())?;
    let health = probe
        .get(format!("{base}/health"))
        .send()
        .await
        .map_err(|e| format!("mflux unreachable: {e}"))?;
    if !health.status().is_success() {
        return Err(format!("mflux health {}", health.status()));
    }

    let (width, height) = dimensions_for(&spec.format);
    let client = reqwest::Client::builder()
        .timeout(MFLUX_CALL)
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(format!("{base}/v1/images/generate"))
        .json(&serde_json::json!({
            "prompt": prompt_for(spec),
            "width": width,
            "height": height,
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("mflux generate {}", resp.status()));
    }
    let ctype = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = if ctype.starts_with("image/") {
        resp.bytes().await.map_err(|e| e.to_string())?.to_vec()
    } else {
        let v: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        if let Some(b64) = v.get("image_base64").and_then(|x| x.as_str()) {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|e| e.to_string())?
        } else if let Some(url) = v.get("url").and_then(|x| x.as_str()) {
            let img = client
                .get(url)
                .send()
                .await
                .map_err(|e| e.to_string())?
                .bytes()
                .await
                .map_err(|e| e.to_string())?;
            img.to_vec()
        } else {
            return Err("mflux response had neither image bytes nor url".into());
        }
    };
    let path = dir.join("still.png");
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn compose_writes_png_from_project_fields_only() {
        let dir = TempDir::new().unwrap();
        let spec = StillSpec {
            title: "Search is live".into(),
            body: "Filter by date.".into(),
            project_name: "Example App".into(),
            brand: ProjectBrand {
                bg: "#102030".into(),
                fg: "#EEF2F6".into(),
                accent: "#3D8B6E".into(),
                ..Default::default()
            },
            format: "text".into(),
            feedback: String::new(),
        };
        let path = compose_still(&spec, dir.path()).unwrap();
        assert!(path.ends_with("still.png"));
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"\x89PNG"));
        assert!(dir.path().join("still.svg").is_file());
        // Fallback palette must not appear when the project supplied a kit.
        assert_ne!(parse_hex("#102030").unwrap(), (17, 17, 17));
    }

    #[test]
    fn prompt_includes_this_project_not_a_builtin_brand() {
        let spec = StillSpec {
            title: "Hook".into(),
            body: "Body".into(),
            project_name: "Example App".into(),
            brand: ProjectBrand {
                voice: "Dry, specific.".into(),
                donts: vec!["stock handshakes".into()],
                ..Default::default()
            },
            format: "text".into(),
            feedback: "darker, more space".into(),
        };
        let p = prompt_for(&spec);
        assert!(p.contains("Example App"));
        assert!(p.contains("Dry, specific."));
        assert!(p.contains("stock handshakes"));
        assert!(p.contains("darker, more space"));
        assert!(!p.to_lowercase().contains("permagent"));
    }
}
