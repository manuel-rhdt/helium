use super::properties::computed::PixelLength;

#[derive(Debug, Copy, Clone)]
pub struct Device {
    pub width: PixelLength,
    pub height: PixelLength,
}
