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

pub fn decode_image(bytes: &[u8]) -> Result<DecodedImage> {
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
