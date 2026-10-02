/// A decoded image: tightly packed, non-premultiplied RGBA8.
#[derive(Debug, Clone)]
pub struct ImageData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Decodes PNG or JPEG bytes (format is sniffed from the content).
pub fn decode_image(bytes: &[u8]) -> Result<ImageData, String> {
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let rgba = img.to_rgba8();
    Ok(ImageData {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}
