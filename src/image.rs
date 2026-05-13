use std::collections::HashMap;
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
}

impl ImageStore {
    pub fn insert(&mut self, url: String, image: DecodedImage) {
        self.images.insert(url, image);
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
