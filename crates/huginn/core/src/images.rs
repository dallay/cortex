//! Bounded, magic-byte-validated image attachments for in-memory chat messages.
use crate::{AgentError, Result};
use std::fmt;

/// Maximum raw byte size accepted for a single attached image (5 MiB).
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
/// Maximum number of images attached to a single user message.
pub const MAX_IMAGES_PER_MESSAGE: usize = 4;
/// Maximum total raw image bytes held in memory for one active session (16 MiB).
pub const MAX_SESSION_IMAGE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    WebP,
}

impl ImageFormat {
    pub const fn mime_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::WebP => "image/webp",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
            Self::WebP => "WebP",
        }
    }

    /// Detect supported image format strictly from magic bytes (never file extension).
    pub fn detect_magic(bytes: &[u8]) -> Option<Self> {
        if bytes.len() >= 8 && bytes[..8] == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
            return Some(Self::Png);
        }
        if bytes.len() >= 3 && bytes[..3] == [0xFF, 0xD8, 0xFF] {
            return Some(Self::Jpeg);
        }
        if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            return Some(Self::WebP);
        }
        None
    }
}

/// In-memory validated image attachment. Never serialized to SQLite or logged verbatim.
#[derive(Clone, PartialEq, Eq)]
pub struct ImageAttachment {
    pub format: ImageFormat,
    pub bytes: Vec<u8>,
    pub dimensions: Option<(u32, u32)>,
    pub label: Option<String>,
}

impl fmt::Debug for ImageAttachment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageAttachment")
            .field("format", &self.format)
            .field("byte_len", &self.bytes.len())
            .field("dimensions", &self.dimensions)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl ImageAttachment {
    /// Validate size and magic bytes, returning a bounded in-memory attachment.
    pub fn from_bytes(bytes: Vec<u8>, label: Option<String>) -> Result<Self> {
        if bytes.is_empty() {
            return Err(AgentError::Configuration(
                "image attachment is empty".into(),
            ));
        }
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(AgentError::Configuration(format!(
                "image exceeds the {} MiB per-image limit",
                MAX_IMAGE_BYTES / (1024 * 1024)
            )));
        }
        let format = ImageFormat::detect_magic(&bytes).ok_or_else(|| {
            AgentError::Configuration(
                "unsupported image format; only PNG, JPEG, and WebP are supported".into(),
            )
        })?;
        let dimensions = parse_dimensions(format, &bytes);
        Ok(Self {
            format,
            bytes,
            dimensions,
            label,
        })
    }

    /// Compact human-readable attachment chip for composer preview.
    pub fn chip_label(&self, one_based_index: usize) -> String {
        let size = format_size(self.bytes.len());
        let dims = self
            .dimensions
            .map(|(w, h)| format!(" {w}x{h}"))
            .unwrap_or_default();
        format!(
            "[Image {one_based_index}: {}{dims}, {size}]",
            self.format.label()
        )
    }
}

fn format_size(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{} KB", bytes.div_ceil(1024))
    } else {
        format!("{bytes} B")
    }
}

fn parse_dimensions(format: ImageFormat, bytes: &[u8]) -> Option<(u32, u32)> {
    match format {
        ImageFormat::Png => {
            // PNG IHDR starts at byte 16 (width: u32 BE, height: u32 BE).
            if bytes.len() >= 24 && &bytes[12..16] == b"IHDR" {
                let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
                let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
                return (w > 0 && h > 0).then_some((w, h));
            }
            None
        }
        ImageFormat::Jpeg => parse_jpeg_dimensions(bytes),
        ImageFormat::WebP => parse_webp_dimensions(bytes),
    }
}

fn parse_jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    while i + 9 < bytes.len() {
        if bytes[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = bytes[i + 1];
        i += 2;
        if marker == 0xD9 || marker == 0xDA {
            break;
        }
        if matches!(
            marker,
            0xC0 | 0xC1
                | 0xC2
                | 0xC3
                | 0xC5
                | 0xC6
                | 0xC7
                | 0xC9
                | 0xCA
                | 0xCB
                | 0xCD
                | 0xCE
                | 0xCF
        ) && i + 7 <= bytes.len()
        {
            let h = u32::from(u16::from_be_bytes([bytes[i + 3], bytes[i + 4]]));
            let w = u32::from(u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]));
            return (w > 0 && h > 0).then_some((w, h));
        }
        if i + 2 > bytes.len() {
            break;
        }
        let len = usize::from(u16::from_be_bytes([bytes[i], bytes[i + 1]]));
        if len < 2 {
            break;
        }
        i = i.saturating_add(len);
    }
    None
}

fn parse_webp_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() >= 30 && &bytes[12..16] == b"VP8X" {
        let w = 1 + u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]);
        let h = 1 + u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]);
        return Some((w, h));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 2, 0, 0, 0]);
        bytes
    }

    #[test]
    fn validates_magic_bytes_dimensions_and_redacts_debug() {
        let img = ImageAttachment::from_bytes(minimal_png(640, 480), None).unwrap();
        assert_eq!(img.format, ImageFormat::Png);
        assert_eq!(img.dimensions, Some((640, 480)));
        assert_eq!(img.chip_label(1), "[Image 1: PNG 640x480, 29 B]");
        let debug = format!("{img:?}");
        assert!(debug.contains("byte_len: 29"));
        assert!(!debug.contains("137, 80, 78"));
    }

    #[test]
    fn rejects_unsupported_and_oversized_payloads() {
        assert!(ImageAttachment::from_bytes(b"not an image".to_vec(), None).is_err());
        let mut huge = minimal_png(1, 1);
        huge.resize(MAX_IMAGE_BYTES + 1, 0);
        assert!(ImageAttachment::from_bytes(huge, None).is_err());
    }

    #[test]
    fn session_for_persistence_strips_image_bytes_and_preserves_marker() {
        let mut session = crate::Session::new(std::path::PathBuf::from("/tmp"));
        let img = ImageAttachment::from_bytes(minimal_png(16, 16), None).unwrap();
        session
            .messages
            .push(crate::Message::user_with_images("describe this", vec![img]));
        assert_eq!(session.in_memory_image_bytes(), 29);

        let persisted = session.for_persistence();
        assert_eq!(persisted.in_memory_image_bytes(), 0);
        assert!(persisted.messages[0].images.is_empty());
        assert!(persisted.messages[0]
            .content
            .contains("[Image attachment omitted from persisted history]"));

        let json = serde_json::to_string(&persisted).unwrap();
        assert!(!json.contains("images"));
    }

    #[test]
    fn vision_capability_gate_accepts_verified_openai_models_only() {
        use crate::providers::supports_openai_vision;

        assert!(supports_openai_vision(
            "openai",
            Some("https://api.openai.com/v1"),
            "gpt-4o"
        ));
        assert!(supports_openai_vision(
            "openai",
            Some("https://api.openai.com/v1"),
            "gpt-5"
        ));
        assert!(supports_openai_vision(
            "openai",
            Some("https://api.openai.com/v1"),
            "o4-mini"
        ));
        assert!(!supports_openai_vision(
            "openai",
            Some("https://api.openai.com/v1"),
            "o3-mini"
        ));
        assert!(!supports_openai_vision(
            "openai",
            Some("https://custom.example.com/v1"),
            "gpt-4o"
        ));
        assert!(!supports_openai_vision(
            "ollama",
            Some("http://127.0.0.1:11434"),
            "llava"
        ));
    }
}
