//! Image attachment handling — detection, reading, and encoding.

use std::path::{Path, PathBuf};

/// Supported image extensions.
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif"];

/// Maximum image file size (20 MB).
const MAX_IMAGE_SIZE: u64 = 20 * 1024 * 1024;

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
pub fn read_image_attachment(path: &Path) -> Result<Attachment, String> {
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

    let display_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("image")
        .to_string();

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
) -> Result<Attachment, String> {
    let w: u32 = width
        .try_into()
        .map_err(|_| format!("Image dimensions too large: {width}x{height}"))?;
    let h: u32 = height
        .try_into()
        .map_err(|_| format!("Image dimensions too large: {width}x{height}"))?;

    let expected_len = (width as u64) * (height as u64) * 4;
    if expected_len > MAX_IMAGE_SIZE {
        return Err(format!(
            "Image data too large: {} for {width}x{height}",
            format_size(expected_len as usize),
        ));
    }
    if rgba.len() != expected_len as usize {
        return Err(format!(
            "RGBA data length mismatch: expected {expected_len} bytes for {width}x{height}, got {}",
            rgba.len()
        ));
    }

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

#[allow(dead_code)] // used in app.rs handle_paste (wired in next commit)
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

        let path = Path::new(unquoted);
        if !unquoted.is_empty() && is_image_path(path) && path.exists() {
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
        let result = read_image_attachment(Path::new("/nonexistent/photo.png"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Cannot read"));
    }

    #[test]
    fn read_image_attachment_wrong_extension() {
        let result = read_image_attachment(Path::new("file.txt"));
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

        let att = read_image_attachment(&img_path).unwrap();
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
        let att = attachment_from_rgba(rgba, 2, 2).unwrap();
        assert_eq!(att.media_type, "image/png");
        assert!(att.display_name.starts_with("clipboard-"));
        assert!(att.display_name.ends_with(".png"));
        // Verify PNG magic bytes
        assert_eq!(&att.data[..4], &[0x89, b'P', b'N', b'G']);
    }

    #[test]
    fn attachment_from_rgba_rejects_oversized_dimensions() {
        let result = attachment_from_rgba(vec![], usize::MAX, 1);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("too large"));
    }

    #[test]
    fn attachment_from_rgba_rejects_mismatched_data() {
        // 2x2 image needs 16 bytes of RGBA, but we give 4
        let result = attachment_from_rgba(vec![0, 0, 0, 0], 2, 2);
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
    fn try_attach_plain_text_unchanged() {
        let (paths, remainder) = try_attach_pasted_images("just some regular text");
        assert!(paths.is_empty());
        assert_eq!(remainder, "just some regular text");
    }

    #[test]
    fn extract_at_image_paths_finds_images() {
        let text = "Check @screenshot.png and also @diagram.webp but not @readme.md";
        let paths = extract_at_image_paths(text);
        assert_eq!(paths, vec!["screenshot.png", "diagram.webp"]);
    }
}
