use std::collections::{HashMap, HashSet};
use std::io::Cursor;

use crate::error::{BrowserError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct ImageStore {
    images: HashMap<String, DecodedImage>,
    /// URLs that were fetched but could not be decoded (an unsupported format
    /// such as SVG, or a 404 body). Without this, callers that only cache
    /// successes re-fetch the same broken URL once per element referencing it:
    /// one SVG icon on a Wikipedia article cost 274 repeat requests.
    failed: HashSet<String>,
}

impl ImageStore {
    pub fn insert(&mut self, url: String, image: DecodedImage) {
        self.failed.remove(&url);
        self.images.insert(url, image);
    }

    /// Record that `url` was tried and did not yield a usable image.
    pub fn mark_failed(&mut self, url: String) {
        if !self.images.contains_key(&url) {
            self.failed.insert(url);
        }
    }

    /// Whether this URL has already been fetched, successfully or not. Callers
    /// use this to decide whether to hit the network, rather than `get`, which
    /// only reports successes.
    pub fn was_attempted(&self, url: &str) -> bool {
        self.images.contains_key(url) || self.failed.contains(url)
    }

    pub fn get(&self, url: &str) -> Option<&DecodedImage> {
        self.images.get(url)
    }
}

/// Decode the payload of a `data:` URL, or `None` if this is not one.
///
/// These carry a good part of a modern page's imagery -- icon sets are inlined
/// rather than fetched -- and nothing here understood them, so every such image
/// was resolved as a relative URL and fetched from the site's own host.
pub fn decode_data_url(url: &str) -> Option<Vec<u8>> {
    let rest = url.strip_prefix("data:").or_else(|| url.strip_prefix("DATA:"))?;
    let (metadata, payload) = rest.split_once(',')?;
    if metadata.to_ascii_lowercase().ends_with(";base64") {
        decode_base64(payload)
    } else {
        Some(decode_percent(payload))
    }
}

fn decode_percent(input: &str) -> Vec<u8> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = |b: u8| -> Option<u8> {
                match b {
                    b'0'..=b'9' => Some(b - b'0'),
                    b'a'..=b'f' => Some(b - b'a' + 10),
                    b'A'..=b'F' => Some(b - b'A' + 10),
                    _ => None,
                }
            };
            if let (Some(hi), Some(lo)) = (hex(bytes[index + 1]), hex(bytes[index + 2])) {
                out.push(hi * 16 + lo);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    out
}

fn decode_base64(input: &str) -> Option<Vec<u8>> {
    fn value(byte: u8) -> Option<u32> {
        match byte {
            b'A'..=b'Z' => Some((byte - b'A') as u32),
            b'a'..=b'z' => Some((byte - b'a') as u32 + 26),
            b'0'..=b'9' => Some((byte - b'0') as u32 + 52),
            // Accept the URL-safe alphabet too; both appear in the wild.
            b'+' | b'-' => Some(62),
            b'/' | b'_' => Some(63),
            _ => None,
        }
    }

    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut accumulator = 0_u32;
    let mut bits = 0_u32;
    for byte in input.bytes() {
        if byte == b'=' {
            break;
        }
        if byte.is_ascii_whitespace() || byte == b'%' {
            continue;
        }
        let Some(value) = value(byte) else {
            return None;
        };
        accumulator = (accumulator << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((accumulator >> bits) as u8);
        }
    }
    Some(out)
}

/// Whether these bytes look like an SVG document.
fn looks_like_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(2048)];
    let text = String::from_utf8_lossy(head);
    let text = text.trim_start_matches('\u{feff}').trim_start();
    (text.starts_with("<?xml") || text.starts_with("<!--") || text.starts_with("<svg"))
        && text.contains("<svg")
}

pub fn decode_image(bytes: &[u8]) -> Result<DecodedImage> {
    if looks_like_svg(bytes) {
        let source = String::from_utf8_lossy(bytes);
        return crate::svg::rasterize(&source)
            .ok_or_else(|| BrowserError::message("unsupported SVG".to_string()));
    }

    let reader = ::image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| BrowserError::message(error.to_string()))?;
    let decoded = reader
        .decode()
        .map_err(|error| BrowserError::message(error.to_string()))?
        .to_rgba8();

    Ok(DecodedImage {
        width: decoded.width(),
        height: decoded.height(),
        rgba: decoded.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::{DecodedImage, ImageStore};

    /// Only successes used to be recorded, so an undecodable image (SVG, or a
    /// 404 page body) was re-fetched once per element that referenced it.
    #[test]
    fn failed_fetches_are_remembered_so_they_are_not_retried() {
        let mut store = ImageStore::default();
        let url = "https://example.com/icon.svg";

        assert!(!store.was_attempted(url));
        store.mark_failed(url.to_string());
        assert!(store.was_attempted(url), "a failed fetch must not be retried");
        assert!(store.get(url).is_none(), "it still has no decoded image");
    }

    /// A later success replaces the failure rather than being shadowed by it.
    #[test]
    fn a_success_clears_an_earlier_failure() {
        let mut store = ImageStore::default();
        let url = "https://example.com/icon.png";
        store.mark_failed(url.to_string());
        store.insert(
            url.to_string(),
            DecodedImage { width: 1, height: 1, rgba: vec![0, 0, 0, 255] },
        );
        assert!(store.was_attempted(url));
        assert!(store.get(url).is_some());
    }

    #[test]
    fn stores_and_reads_images() {
        let mut store = ImageStore::default();
        store.insert(
            "https://example.com/demo.png".to_string(),
            DecodedImage {
                width: 2,
                height: 1,
                rgba: vec![255, 0, 0, 255, 0, 255, 0, 255],
            },
        );

        let image = store
            .get("https://example.com/demo.png")
            .expect("image should be stored");

        assert_eq!(image.width, 2);
        assert_eq!(image.height, 1);
    }
}
