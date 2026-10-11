//! Screenshot image validation for non-terminal frontends. PNG/JPEG only;
//! byte-size, dimension, and allocation caps are enforced before and during
//! a full decode. Returns the validated *original* bytes (never a
//! re-encoded thumbnail) so a GUI can render the real image.

use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::Value;
use std::io::Cursor;

/// Decoded image byte cap (8 MiB).
pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
/// Base64 inflates ~4/3; bound the encoded text before decoding.
pub const MAX_BASE64_LEN: usize = MAX_IMAGE_BYTES.div_ceil(3) * 4;
/// Largest accepted width/height.
pub const MAX_DIMENSION: u32 = 4096;
/// Total decoder allocation cap.
pub const MAX_ALLOC_BYTES: u64 = 64 * 1024 * 1024;

/// An image that survived every cap and a full decode; `bytes` are the
/// caller's original encoded PNG/JPEG, untouched.
#[derive(Clone, Debug)]
pub struct ValidatedImage {
    pub mime: String,
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

fn format_for(mime: &str) -> Result<::image::ImageFormat> {
    match mime {
        "image/png" => Ok(::image::ImageFormat::Png),
        "image/jpeg" => Ok(::image::ImageFormat::Jpeg),
        _ => anyhow::bail!("unsupported image MIME {mime:?}"),
    }
}

/// Validate raw PNG/JPEG bytes. The size cap applies before decode;
/// dimension and allocation limits apply during the mandatory decode.
pub fn validate_bytes(mime: &str, bytes: &[u8]) -> Result<ValidatedImage> {
    let format = format_for(mime)?;
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_IMAGE_BYTES,
        "image exceeds 8 MiB"
    );
    let mut reader = ::image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = ::image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_ALLOC_BYTES);
    reader.limits(limits);
    let decoded = reader.decode().context("cannot decode image")?;
    ensure!(decoded.width() > 0 && decoded.height() > 0, "empty image");
    Ok(ValidatedImage {
        mime: mime.to_string(),
        bytes: bytes.to_vec(),
        width: decoded.width(),
        height: decoded.height(),
    })
}

/// Validate a base64-encoded PNG/JPEG. The encoded-length bound is checked
/// before any decoding work.
pub fn validate_base64(mime: &str, encoded: &str) -> Result<ValidatedImage> {
    ensure!(encoded.len() <= MAX_BASE64_LEN, "image base64 too large");
    let bytes = STANDARD.decode(encoded).context("invalid image base64")?;
    ensure!(bytes.len() <= MAX_IMAGE_BYTES, "image exceeds 8 MiB");
    validate_bytes(mime, &bytes)
}

/// Extract and validate the image inside an MCP `call` tool result (the
/// `content[]` entry with `type: "image"`) — the same envelope as the TUI
/// preview decoder, but returning real bytes for an `<img>`-style frontend.
pub fn decode_tool_result(result: &Value) -> Result<ValidatedImage> {
    let result = result.get("result").unwrap_or(result);
    ensure!(
        !result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        "screenshot tool failed"
    );
    let content = result
        .get("content")
        .and_then(Value::as_array)
        .context("screenshot missing content")?;
    let image = content
        .iter()
        .find(|c| c.get("type").and_then(Value::as_str) == Some("image"))
        .context("screenshot missing image")?;
    let mime = image
        .get("mimeType")
        .and_then(Value::as_str)
        .context("missing image MIME")?;
    let encoded = image
        .get("data")
        .and_then(Value::as_str)
        .context("missing screenshot bytes")?;
    validate_base64(mime, encoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        ::image::RgbImage::new(width, height)
            .write_to(&mut bytes, ::image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    #[test]
    fn valid_png_and_jpeg_return_original_bytes() {
        let bytes = png(2, 2);
        let image = validate_bytes("image/png", &bytes).unwrap();
        assert_eq!(image.bytes, bytes, "validated bytes must be the original");
        assert_eq!((image.width, image.height), (2, 2));
        let mut jpeg = Cursor::new(Vec::new());
        ::image::RgbImage::new(3, 1)
            .write_to(&mut jpeg, ::image::ImageFormat::Jpeg)
            .unwrap();
        assert!(validate_bytes("image/jpeg", &jpeg.into_inner()).is_ok());
        assert!(validate_bytes("image/jpeg", &bytes).is_err());
    }

    #[test]
    fn rejects_mime_empty_oversized_dim_and_corruption() {
        assert!(validate_bytes("image/gif", &png(1, 1)).is_err());
        assert!(validate_bytes("image/webp", &png(1, 1)).is_err());
        assert!(validate_bytes("image/png", &[]).is_err());
        assert!(validate_bytes("image/png", b"not an image").is_err());
        assert!(validate_bytes("image/png", &png(MAX_DIMENSION + 1, 1)).is_err());
        assert!(validate_bytes("image/png", &png(1, MAX_DIMENSION + 1)).is_err());
        let mut oversized = png(1, 1);
        oversized.resize(MAX_IMAGE_BYTES + 1, 0);
        assert!(validate_bytes("image/png", &oversized).is_err());
    }

    #[test]
    fn base64_bound_is_checked_before_decode() {
        let encoded = STANDARD.encode(png(2, 2));
        assert_eq!(validate_base64("image/png", &encoded).unwrap().width, 2);
        assert!(validate_base64("image/png", "!!!").is_err());
        let huge = STANDARD.encode(vec![0u8; MAX_IMAGE_BYTES + 3]);
        assert!(huge.len() > MAX_BASE64_LEN);
        assert!(validate_base64("image/png", &huge).is_err());
    }

    #[test]
    fn tool_result_envelope_is_extracted_and_validated() {
        let encoded = STANDARD.encode(png(2, 2));
        let ok =
            serde_json::json!({"content":[{"type":"image","mimeType":"image/png","data":encoded}]});
        assert_eq!(decode_tool_result(&ok).unwrap().mime, "image/png");
        for bad in [
            serde_json::json!({"isError": true, "content": []}),
            serde_json::json!({"content": []}),
            serde_json::json!({"content": [{"type": "text", "text": "hi"}]}),
            serde_json::json!({"content": [{"type": "image", "mimeType": "image/gif", "data": encoded}]}),
            serde_json::json!({"content": [{"type": "image", "mimeType": "image/png", "data": "invalid"}]}),
            serde_json::json!({}),
        ] {
            assert!(decode_tool_result(&bad).is_err(), "must reject {bad}");
        }
    }
}
