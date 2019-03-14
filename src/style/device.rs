
use super::properties::computed::CssFloat;

#[derive(Debug, Copy, Clone)]
pub struct Device {
    pub width: CssFloat,
    pub height: CssFloat,
}