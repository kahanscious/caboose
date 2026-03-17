//! Image attachment handling — detection, reading, and encoding.

use std::path::{Path, PathBuf};

/// Supported image extensions.
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif"];

/// Maximum image file size (20 MB).
const MAX_IMAGE_SIZE: u64 = 20 * 1024 * 1024;

/// Compress threshold — images under this size skip compression.
/// 3.75 MB raw → ~5 MB base64, staying under Anthropic's API limit.
const COMPRESS_THRESHOLD: usize = 3_932_160;

/// A pending file attachment.
#[derive(Debug, Clone)]
pub struct Attachment {
    pub path: PathBuf,
    pub media_type: String,
    pub data: Vec<u8>,
    pub display_name: String,
}

/// Check if a file path has an image extension.
pub fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// Determine MIME type from file extension.
pub fn media_type_from_ext(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("image/png".to_string()),
        "jpg" | "jpeg" => Some("image/jpeg".to_string()),
        "webp" => Some("image/webp".to_string()),
        "gif" => Some("image/gif".to_string()),
        _ => None,
    }
}

/// Read an image file and create an Attachment.
pub fn read_image_attachment(path: &Path, config: &crate::config::schema::ImagesConfig) -> Result<Attachment, String> {
    if !is_image_path(path) {
        return Err(format!("Not a supported image format: {}", path.display()));
    }

    let metadata =
        std::fs::metadata(path).map_err(|e| format!("Cannot read {}: {}", path.display(), e))?;

    if metadata.len() > MAX_IMAGE_SIZE {
        return Err(format!(
            "Image too large: {} (max {})",
            format_size(metadata.len() as usize),
            format_size(MAX_IMAGE_SIZE as usize),
        ));
    }

    let data =
        std::fs::read(path).map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    let media_type = media_type_from_ext(path)
        .ok_or_else(|| format!("Unknown media type for {}", path.display()))?;

    let _original_size = data.len();
    let _original_media_type = media_type.clone();

    // Compress/resize if needed
    let (data, media_type) = compress_image(&data, &media_type, config)
        .unwrap_or_else(|_| (data, media_type));

    // Update display name if format changed to JPEG
    let display_name = if media_type == "image/jpeg"
        && !path.extension().is_some_and(|e| {
            let e = e.to_ascii_lowercase();
            e == "jpg" || e == "jpeg"
        })
    {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("image");
        format!("{stem}.jpg")
    } else {
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("image")
            .to_string()
    };

    Ok(Attachment {
        path: path.to_path_buf(),
        media_type,
        data,
        display_name,
    })
}

/// Format file size for display (e.g., "245 KB", "1.2 MB").
pub fn format_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        let mb = bytes as f64 / (1024.0 * 1024.0);
        if mb < 10.0 {
            format!("{:.1} MB", mb)
        } else {
            format!("{:.0} MB", mb)
        }
    }
}

/// Create an Attachment from raw RGBA pixel data (e.g. from clipboard).
/// Encodes the data as PNG. Returns an error if dimensions are invalid.
pub fn attachment_from_rgba(
    rgba: Vec<u8>,
    width: usize,
    height: usize,
    config: &crate::config::schema::ImagesConfig,
) -> Result<Attachment, String> {
    let w: u32 = width
        .try_into()
        .map_err(|_| format!("Image dimensions too large: {width}x{height}"))?;
    let h: u32 = height
        .try_into()
        .map_err(|_| format!("Image dimensions too large: {width}x{height}"))?;

    let expected_len = (width as u64) * (height as u64) * 4;
    if rgba.len() != expected_len as usize {
        return Err(format!(
            "RGBA data length mismatch: expected {expected_len} bytes for {width}x{height}, got {}",
            rgba.len()
        ));
    }

    // Resize RGBA buffer directly if dimensions exceed limit
    // (avoids encode → decode → resize → re-encode round-trip)
    let max_dim = config.max_dimension();
    let (rgba, w, h) = if config.enabled() && (w > max_dim || h > max_dim) {
        let img_buf = image::RgbaImage::from_raw(w, h, rgba)
            .ok_or_else(|| format!("Failed to create image buffer from {w}x{h} RGBA data"))?;
        let dynamic = image::DynamicImage::ImageRgba8(img_buf);
        let resized = dynamic.resize(max_dim, max_dim, image::imageops::FilterType::Lanczos3);
        let resized_rgba = resized.to_rgba8();
        let new_w = resized_rgba.width();
        let new_h = resized_rgba.height();
        (resized_rgba.into_raw(), new_w, new_h)
    } else {
        // Only enforce raw-data size limit when not resizing
        if expected_len > MAX_IMAGE_SIZE {
            return Err(format!(
                "Image data too large: {} for {width}x{height}",
                format_size(expected_len as usize),
            ));
        }
        (rgba, w, h)
    };

    // Encode as PNG
    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| format!("PNG header error: {e}"))?;
        writer
            .write_image_data(&rgba)
            .map_err(|e| format!("PNG encode error: {e}"))?;
    }

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let display_name = format!("clipboard-{timestamp}.png");

    Ok(Attachment {
        path: PathBuf::from(&display_name),
        media_type: "image/png".to_string(),
        data: png_bytes,
        display_name,
    })
}

/// Compress and resize an image if it exceeds size/dimension thresholds.
///
/// Returns (compressed_data, media_type). On decode failure, returns the
/// original data unchanged and logs a warning.
pub fn compress_image(
    data: &[u8],
    media_type: &str,
    config: &crate::config::schema::ImagesConfig,
) -> Result<(Vec<u8>, String), String> {
    use image::ImageFormat;

    // Disabled — passthrough
    if !config.enabled() {
        return Ok((data.to_vec(), media_type.to_string()));
    }

    // GIFs pass through unchanged (would lose animation)
    if media_type == "image/gif" {
        return Ok((data.to_vec(), media_type.to_string()));
    }

    let max_dim = config.max_dimension();

    // Try to decode the image
    let img = match image::load_from_memory(data) {
        Ok(img) => img,
        Err(e) => {
            tracing::warn!("Image decode failed, skipping compression: {e}");
            return Ok((data.to_vec(), media_type.to_string()));
        }
    };

    let (w, h) = (img.width(), img.height());
    let needs_resize = w > max_dim || h > max_dim;
    let needs_compress = data.len() > COMPRESS_THRESHOLD;

    // Step 1: passthrough — small and within dimension limits
    if !needs_resize && !needs_compress {
        return Ok((data.to_vec(), media_type.to_string()));
    }

    // Step 2: resize if dimensions exceed limit (aspect ratio preserved)
    let img = if needs_resize {
        img.resize(max_dim, max_dim, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };

    // Determine if image has alpha
    let has_alpha = matches!(
        img.color(),
        image::ColorType::Rgba8
            | image::ColorType::Rgba16
            | image::ColorType::Rgba32F
            | image::ColorType::La8
            | image::ColorType::La16
    );

    // For size-only triggers (no resize needed), skip re-encoding in original format
    // and go straight to JPEG conversion — re-encoding a large PNG as PNG won't shrink it.
    if !needs_resize && needs_compress && !has_alpha {
        let quality_high = config.jpeg_quality();
        let mut jpeg_buf = Vec::new();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
            &mut jpeg_buf,
            quality_high,
        );
        if let Err(e) = img.write_with_encoder(encoder) {
            tracing::warn!("JPEG encode failed, returning original: {e}");
            return Ok((data.to_vec(), media_type.to_string()));
        }
        if jpeg_buf.len() <= COMPRESS_THRESHOLD {
            return Ok((jpeg_buf, "image/jpeg".to_string()));
        }
        // Try low quality
        let quality_low = config.jpeg_quality_low();
        let mut jpeg_low_buf = Vec::new();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
            &mut jpeg_low_buf,
            quality_low,
        );
        if let Err(e) = img.write_with_encoder(encoder) {
            tracing::warn!("JPEG low-quality encode failed: {e}");
            return Ok((jpeg_buf, "image/jpeg".to_string()));
        }
        return Ok((jpeg_low_buf, "image/jpeg".to_string()));
    }

    // Re-encode in original format after resize
    let format = match media_type {
        "image/png" => ImageFormat::Png,
        "image/jpeg" => ImageFormat::Jpeg,
        "image/webp" => ImageFormat::WebP,
        _ => ImageFormat::Png,
    };

    let mut buf = Vec::new();
    if let Err(e) = img.write_to(&mut std::io::Cursor::new(&mut buf), format) {
        tracing::warn!("Image re-encode failed, returning original: {e}");
        return Ok((data.to_vec(), media_type.to_string()));
    }

    // If under threshold after resize, return in original format
    if buf.len() <= COMPRESS_THRESHOLD {
        return Ok((buf, media_type.to_string()));
    }

    // Step 3: re-encode as JPEG (only if no alpha)
    if has_alpha {
        return Ok((buf, media_type.to_string()));
    }

    let quality_high = config.jpeg_quality();
    let mut jpeg_buf = Vec::new();
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
        &mut jpeg_buf,
        quality_high,
    );
    if let Err(e) = img.write_with_encoder(encoder) {
        tracing::warn!("JPEG encode failed, returning resized original: {e}");
        return Ok((buf, media_type.to_string()));
    }

    if jpeg_buf.len() <= COMPRESS_THRESHOLD {
        return Ok((jpeg_buf, "image/jpeg".to_string()));
    }

    // Try low quality JPEG
    let quality_low = config.jpeg_quality_low();
    let mut jpeg_low_buf = Vec::new();
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
        &mut jpeg_low_buf,
        quality_low,
    );
    if let Err(e) = img.write_with_encoder(encoder) {
        tracing::warn!("JPEG low-quality encode failed: {e}");
        return Ok((jpeg_buf, "image/jpeg".to_string()));
    }

    Ok((jpeg_low_buf, "image/jpeg".to_string()))
}

/// Remove shell-style backslash escapes from a path string.
/// e.g. `Screen\ shot\ 2026.png` → `Screen shot 2026.png`
///
/// Only unescapes `\` before shell-special characters (space, parens, brackets,
/// quotes, etc.) to avoid mangling Windows path separators like `C:\Users\`.
fn unescape_shell_path(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&next) = chars.peek() {
                if " ()[]'\"!#$&;|{}".contains(next) {
                    // Shell escape — drop the backslash, keep the character
                    result.push(chars.next().unwrap());
                } else {
                    // Not a shell escape (likely Windows path separator) — keep as-is
                    result.push(c);
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Detect image file paths in pasted text (e.g. from terminal drag-and-drop).
/// Returns (image_paths_found, remaining_text_to_insert).
pub fn try_attach_pasted_images(paste: &str) -> (Vec<PathBuf>, String) {
    let mut image_paths = Vec::new();
    let mut remaining_lines = Vec::new();

    for line in paste.lines() {
        let trimmed = line.trim();
        // Strip surrounding quotes (some terminals wrap dropped file paths)
        let unquoted = trimmed
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .or_else(|| {
                trimmed
                    .strip_prefix('\'')
                    .and_then(|s| s.strip_suffix('\''))
            })
            .unwrap_or(trimmed);

        // Unescape shell-style backslash sequences (e.g. Warp pastes "Screen\ shot.png")
        let unescaped = unescape_shell_path(unquoted);
        let path = Path::new(&unescaped);

        if !unescaped.is_empty() && is_image_path(path) && path.exists() {
            image_paths.push(path.to_path_buf());
        } else {
            remaining_lines.push(line);
        }
    }

    let remainder = remaining_lines.join("\n");
    (image_paths, remainder)
}

/// Extract bare image file paths from message text (no `@` prefix required).
/// Finds absolute paths to existing image files embedded in the message and returns
/// them along with the text with those paths removed.
pub fn extract_bare_image_paths(text: &str) -> (Vec<PathBuf>, String) {
    let mut image_paths = Vec::new();
    let mut remaining_lines = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        let unescaped = unescape_shell_path(trimmed);
        let path = Path::new(&unescaped);

        // Only match absolute paths to avoid false positives on regular words
        if path.is_absolute() && is_image_path(path) && path.exists() {
            image_paths.push(path.to_path_buf());
        } else {
            remaining_lines.push(line);
        }
    }

    let remainder = remaining_lines.join("\n");
    (image_paths, remainder)
}

/// Extract `@file` references from input text that point to image files.
pub fn extract_at_image_paths(text: &str) -> Vec<String> {
    let mut results = Vec::new();
    for word in text.split_whitespace() {
        if let Some(path_str) = word.strip_prefix('@')
            && !path_str.is_empty()
            && is_image_path(Path::new(path_str))
        {
            results.push(path_str.to_string());
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use crate::config::schema::ImagesConfig;

    fn default_images_config() -> ImagesConfig {
        toml::from_str("").unwrap()
    }

    fn disabled_images_config() -> ImagesConfig {
        toml::from_str("enabled = false").unwrap()
    }

    /// Create a synthetic PNG image of given dimensions (solid red).
    fn make_test_png(width: u32, height: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_fn(width, height, |_, _| {
            image::Rgba([255, 0, 0, 255])
        });
        let mut buf = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut buf);
        img.write_to(&mut cursor, image::ImageFormat::Png).unwrap();
        buf
    }

    /// Create a synthetic JPEG image of given dimensions.
    fn make_test_jpeg(width: u32, height: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(width, height, |_, _| {
            image::Rgb([255, 0, 0])
        });
        let mut buf = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut buf);
        img.write_to(&mut cursor, image::ImageFormat::Jpeg).unwrap();
        buf
    }

    /// Create a synthetic PNG with alpha (transparent pixels).
    fn make_test_png_with_alpha(width: u32, height: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_fn(width, height, |_, _| {
            image::Rgba([255, 0, 0, 128]) // semi-transparent red
        });
        let mut buf = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut buf);
        img.write_to(&mut cursor, image::ImageFormat::Png).unwrap();
        buf
    }

    #[test]
    fn compress_image_passthrough_small() {
        let config = default_images_config();
        let png = make_test_png(100, 100);
        let original_len = png.len();
        let (data, media_type) = compress_image(&png, "image/png", &config).unwrap();
        assert_eq!(data.len(), original_len, "small image should pass through");
        assert_eq!(media_type, "image/png");
    }

    #[test]
    fn compress_image_resizes_oversized() {
        let config = default_images_config();
        let png = make_test_png(3000, 2000);
        let (data, media_type) = compress_image(&png, "image/png", &config).unwrap();
        let decoded = image::load_from_memory(&data).unwrap();
        assert!(decoded.width() <= 2000);
        assert!(decoded.height() <= 2000);
        assert_eq!(media_type, "image/png");
    }

    #[test]
    fn compress_image_preserves_aspect_ratio() {
        let config = default_images_config();
        let png = make_test_png(4000, 2000); // 2:1 aspect ratio
        let (data, _) = compress_image(&png, "image/png", &config).unwrap();
        let decoded = image::load_from_memory(&data).unwrap();
        assert_eq!(decoded.width(), 2000);
        assert_eq!(decoded.height(), 1000);
    }

    #[test]
    fn compress_image_alpha_skips_jpeg() {
        let config = default_images_config();
        let png = make_test_png_with_alpha(3000, 3000);
        let (_, media_type) = compress_image(&png, "image/png", &config).unwrap();
        // Should stay PNG because it has alpha
        assert_eq!(media_type, "image/png");
    }

    #[test]
    fn compress_image_disabled_passthrough() {
        let config = disabled_images_config();
        let png = make_test_png(3000, 3000);
        let original_len = png.len();
        let (data, media_type) = compress_image(&png, "image/png", &config).unwrap();
        assert_eq!(data.len(), original_len, "disabled should passthrough");
        assert_eq!(media_type, "image/png");
    }

    #[test]
    fn compress_image_corrupt_returns_original() {
        let config = default_images_config();
        let garbage = vec![0u8; 100];
        let (data, media_type) = compress_image(&garbage, "image/png", &config).unwrap();
        assert_eq!(data, garbage, "corrupt image should return original");
        assert_eq!(media_type, "image/png");
    }

    #[test]
    fn compress_image_gif_passthrough() {
        let config = default_images_config();
        // GIFs are short-circuited before decode (media_type check), so even
        // invalid GIF data passes through unchanged.
        let gif_bytes = b"GIF89a".to_vec();
        let (data, media_type) = compress_image(&gif_bytes, "image/gif", &config).unwrap();
        assert_eq!(data, gif_bytes, "GIF should pass through unchanged");
        assert_eq!(media_type, "image/gif");
    }

    #[test]
    fn compress_image_jpeg_resizes() {
        let config = default_images_config();
        let jpeg = make_test_jpeg(3000, 2000);
        let (data, media_type) = compress_image(&jpeg, "image/jpeg", &config).unwrap();
        let decoded = image::load_from_memory(&data).unwrap();
        assert!(decoded.width() <= 2000);
        assert!(decoded.height() <= 2000);
        assert_eq!(media_type, "image/jpeg");
    }

    #[test]
    fn compress_image_small_webp_passthrough() {
        let config = default_images_config();
        let img = image::RgbImage::from_fn(10, 10, |_, _| image::Rgb([0, 0, 255]));
        let mut buf = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::WebP).unwrap();
        let original_len = buf.len();
        let (data, media_type) = compress_image(&buf, "image/webp", &config).unwrap();
        assert_eq!(data.len(), original_len);
        assert_eq!(media_type, "image/webp");
    }

    #[test]
    fn is_image_path_recognizes_extensions() {
        assert!(is_image_path(Path::new("photo.png")));
        assert!(is_image_path(Path::new("photo.jpg")));
        assert!(is_image_path(Path::new("photo.jpeg")));
        assert!(is_image_path(Path::new("photo.webp")));
        assert!(is_image_path(Path::new("photo.gif")));
        // Case insensitive
        assert!(is_image_path(Path::new("photo.PNG")));
        assert!(is_image_path(Path::new("photo.JPG")));
        // Non-image extensions
        assert!(!is_image_path(Path::new("file.txt")));
        assert!(!is_image_path(Path::new("file.rs")));
        assert!(!is_image_path(Path::new("noext")));
    }

    #[test]
    fn media_type_from_ext_maps_correctly() {
        assert_eq!(
            media_type_from_ext(Path::new("a.png")),
            Some("image/png".to_string())
        );
        assert_eq!(
            media_type_from_ext(Path::new("a.jpg")),
            Some("image/jpeg".to_string())
        );
        assert_eq!(
            media_type_from_ext(Path::new("a.jpeg")),
            Some("image/jpeg".to_string())
        );
        assert_eq!(
            media_type_from_ext(Path::new("a.webp")),
            Some("image/webp".to_string())
        );
        assert_eq!(
            media_type_from_ext(Path::new("a.gif")),
            Some("image/gif".to_string())
        );
        assert_eq!(media_type_from_ext(Path::new("a.txt")), None);
    }

    #[test]
    fn format_size_display() {
        assert_eq!(format_size(500), "500 B");
        assert_eq!(format_size(1024), "1 KB");
        assert_eq!(format_size(245 * 1024), "245 KB");
        assert_eq!(format_size(1_258_291), "1.2 MB");
        assert_eq!(format_size(20 * 1024 * 1024), "20 MB");
    }

    #[test]
    fn read_image_attachment_nonexistent() {
        let result = read_image_attachment(Path::new("/nonexistent/photo.png"), &default_images_config());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Cannot read"));
    }

    #[test]
    fn read_image_attachment_wrong_extension() {
        let result = read_image_attachment(Path::new("file.txt"), &default_images_config());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Not a supported image format"));
    }

    #[test]
    fn read_image_attachment_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let img_path = dir.path().join("test.png");
        // Write a minimal PNG header
        let mut f = std::fs::File::create(&img_path).unwrap();
        f.write_all(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])
            .unwrap();
        drop(f);

        let att = read_image_attachment(&img_path, &default_images_config()).unwrap();
        assert_eq!(att.media_type, "image/png");
        assert_eq!(att.display_name, "test.png");
        assert_eq!(att.data.len(), 8);
        assert_eq!(att.path, img_path);
    }

    #[test]
    fn attachment_from_rgba_produces_valid_png() {
        // 2x2 RGBA image
        let rgba = vec![
            255, 0, 0, 255, // red
            0, 255, 0, 255, // green
            0, 0, 255, 255, // blue
            255, 255, 0, 255, // yellow
        ];
        let att = attachment_from_rgba(rgba, 2, 2, &default_images_config()).unwrap();
        assert_eq!(att.media_type, "image/png");
        assert!(att.display_name.starts_with("clipboard-"));
        assert!(att.display_name.ends_with(".png"));
        // Verify PNG magic bytes
        assert_eq!(&att.data[..4], &[0x89, b'P', b'N', b'G']);
    }

    #[test]
    fn attachment_from_rgba_rejects_oversized_dimensions() {
        let result = attachment_from_rgba(vec![], usize::MAX, 1, &default_images_config());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("too large"));
    }

    #[test]
    fn attachment_from_rgba_rejects_mismatched_data() {
        // 2x2 image needs 16 bytes of RGBA, but we give 4
        let result = attachment_from_rgba(vec![0, 0, 0, 0], 2, 2, &default_images_config());
        assert!(result.is_err());
    }

    #[test]
    fn try_attach_single_image_path() {
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("photo.png");
        std::fs::write(&img, &[0x89, b'P', b'N', b'G']).unwrap();

        let (paths, remainder) = try_attach_pasted_images(img.to_str().unwrap());
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], img);
        assert!(remainder.is_empty());
    }

    #[test]
    fn try_attach_multiple_image_paths() {
        let dir = tempfile::tempdir().unwrap();
        let img1 = dir.path().join("a.png");
        let img2 = dir.path().join("b.jpg");
        std::fs::write(&img1, &[0x89]).unwrap();
        std::fs::write(&img2, &[0xFF]).unwrap();

        let paste = format!("{}\n{}", img1.display(), img2.display());
        let (paths, remainder) = try_attach_pasted_images(&paste);
        assert_eq!(paths.len(), 2);
        assert!(remainder.is_empty());
    }

    #[test]
    fn try_attach_mixed_paths_and_text() {
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("photo.png");
        std::fs::write(&img, &[0x89]).unwrap();

        let paste = format!("hello world\n{}\nsome other text", img.display());
        let (paths, remainder) = try_attach_pasted_images(&paste);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], img);
        assert_eq!(remainder, "hello world\nsome other text");
    }

    #[test]
    fn try_attach_nonexistent_image_path_falls_through() {
        let (paths, remainder) = try_attach_pasted_images("/nonexistent/photo.png");
        assert!(paths.is_empty());
        assert_eq!(remainder, "/nonexistent/photo.png");
    }

    #[test]
    fn try_attach_non_image_path_falls_through() {
        let dir = tempfile::tempdir().unwrap();
        let rs_file = dir.path().join("main.rs");
        std::fs::write(&rs_file, "fn main() {}").unwrap();

        let (paths, remainder) = try_attach_pasted_images(rs_file.to_str().unwrap());
        assert!(paths.is_empty());
        assert_eq!(remainder, rs_file.to_str().unwrap());
    }

    #[test]
    fn try_attach_quoted_path() {
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("photo.png");
        std::fs::write(&img, &[0x89]).unwrap();

        let paste = format!("\"{}\"", img.display());
        let (paths, remainder) = try_attach_pasted_images(&paste);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], img);
        assert!(remainder.is_empty());
    }

    #[test]
    fn try_attach_single_quoted_path() {
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("photo.png");
        std::fs::write(&img, &[0x89]).unwrap();

        let paste = format!("'{}'", img.display());
        let (paths, remainder) = try_attach_pasted_images(&paste);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], img);
        assert!(remainder.is_empty());
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn try_attach_shell_escaped_path() {
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("Screen shot 2026.png");
        std::fs::write(&img, &[0x89]).unwrap();

        // Warp and other terminals paste paths with backslash-escaped spaces
        let escaped = format!("{}/Screen\\ shot\\ 2026.png", dir.path().display());
        let (paths, remainder) = try_attach_pasted_images(&escaped);
        assert_eq!(paths.len(), 1, "should detect shell-escaped image path");
        assert_eq!(paths[0], img);
        assert!(remainder.is_empty());
    }

    #[test]
    fn unescape_shell_path_works() {
        assert_eq!(unescape_shell_path("Screen\\ shot.png"), "Screen shot.png");
        assert_eq!(
            unescape_shell_path("no\\ spaces\\ here.png"),
            "no spaces here.png"
        );
        assert_eq!(unescape_shell_path("noescape.png"), "noescape.png");
        assert_eq!(
            unescape_shell_path("/path/to/Screen\\ shot\\ \\(1\\).png"),
            "/path/to/Screen shot (1).png"
        );
        // Windows path separators should be preserved
        assert_eq!(
            unescape_shell_path("C:\\Users\\test\\photo.png"),
            "C:\\Users\\test\\photo.png"
        );
    }

    #[test]
    fn try_attach_plain_text_unchanged() {
        let (paths, remainder) = try_attach_pasted_images("just some regular text");
        assert!(paths.is_empty());
        assert_eq!(remainder, "just some regular text");
    }

    #[test]
    fn extract_bare_image_path_from_message() {
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("screenshot.png");
        std::fs::write(&img, &[0x89]).unwrap();

        let msg = format!("{}\nwhat's in this image", img.display());
        let (paths, remainder) = extract_bare_image_paths(&msg);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], img);
        assert_eq!(remainder, "what's in this image");
    }

    #[test]
    fn extract_bare_ignores_relative_paths() {
        // Relative paths should NOT be matched to avoid false positives
        let (paths, remainder) = extract_bare_image_paths("check screenshot.png please");
        assert!(paths.is_empty());
        assert_eq!(remainder, "check screenshot.png please");
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn extract_bare_shell_escaped_path() {
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("Screen shot 2026.png");
        std::fs::write(&img, &[0x89]).unwrap();

        let msg = format!(
            "{}/Screen\\ shot\\ 2026.png\ndescribe this",
            dir.path().display()
        );
        let (paths, remainder) = extract_bare_image_paths(&msg);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], img);
        assert_eq!(remainder, "describe this");
    }

    #[test]
    fn extract_at_image_paths_finds_images() {
        let text = "Check @screenshot.png and also @diagram.webp but not @readme.md";
        let paths = extract_at_image_paths(text);
        assert_eq!(paths, vec!["screenshot.png", "diagram.webp"]);
    }

    #[test]
    fn read_image_attachment_compresses_large() {
        let dir = tempfile::tempdir().unwrap();
        let img_path = dir.path().join("big.png");
        let png = make_test_png(3000, 2000);
        std::fs::write(&img_path, &png).unwrap();

        let config = default_images_config();
        let att = read_image_attachment(&img_path, &config).unwrap();
        let decoded = image::load_from_memory(&att.data).unwrap();
        assert!(decoded.width() <= 2000);
        assert!(decoded.height() <= 2000);
    }

    #[test]
    fn attachment_from_rgba_resizes_large_clipboard() {
        let config = default_images_config();
        let width = 3000usize;
        let height = 2000usize;
        let rgba = vec![255u8; width * height * 4];
        let att = attachment_from_rgba(rgba, width, height, &config).unwrap();
        let decoded = image::load_from_memory(&att.data).unwrap();
        assert!(decoded.width() <= 2000);
        assert!(decoded.height() <= 2000);
        assert_eq!(att.media_type, "image/png");
    }
}
