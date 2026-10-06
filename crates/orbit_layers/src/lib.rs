//! Layered image import: PSD, Krita `.kra` and OpenRaster `.ora` files are
//! read into a [`LayeredDoc`] (raster layers, groups, visibility, canvas
//! positions), then turned into slots and meshes of a 2D rig by
//! [`import_into_rig`].
//!
//! Editor / project tool only (the runtime never links this crate). Supported:
//! 8 and 16-bit RGB(A) raster layers, groups. Anything else (adjustment and
//! text layers, masks, blend modes, smart objects...) is skipped or ignored
//! with a warning, never a failure.

mod kra;
mod ora;
mod psd;
mod rig;
mod xml;

pub use rig::{ImportOptions, ImportReport, import_into_rig};

/// A raster layer, RGBA8 straight alpha, placed on the canvas.
#[derive(Clone, Debug)]
pub struct Layer {
    pub name: String,
    /// Enclosing groups, outermost first (indices into [`LayeredDoc::groups`]).
    pub groups: Vec<usize>,
    /// The layer's own visibility flag (a hidden group hides its layers: see
    /// [`LayeredDoc::is_visible`]).
    pub visible: bool,
    /// 0..=1.
    pub opacity: f32,
    /// Top-left corner on the canvas, pixels (may be outside the canvas).
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct Group {
    pub name: String,
    pub parent: Option<usize>,
    pub visible: bool,
}

#[derive(Clone, Debug, Default)]
pub struct LayeredDoc {
    pub width: u32,
    pub height: u32,
    /// Bottom to top.
    pub layers: Vec<Layer>,
    pub groups: Vec<Group>,
    pub warnings: Vec<String>,
}

impl LayeredDoc {
    /// Whether the layer and all its groups are visible.
    pub fn is_visible(&self, layer: &Layer) -> bool {
        layer.visible && layer.groups.iter().all(|g| self.groups[*g].visible)
    }
}

/// Reads a layered file; the format is told from its content (`8BPS`, or a zip
/// holding `stack.xml` / `maindoc.xml`).
pub fn parse(bytes: &[u8]) -> Result<LayeredDoc, String> {
    let doc = if bytes.starts_with(b"8BPS") {
        psd::parse(bytes)?
    } else if bytes.starts_with(b"PK") {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("not a layered file: {e}"))?;
        if zip.by_name("stack.xml").is_ok() {
            ora::parse(&mut zip)?
        } else if zip.by_name("maindoc.xml").is_ok() {
            kra::parse(&mut zip)?
        } else {
            return Err("this zip is neither an OpenRaster (.ora) nor a Krita (.kra) file".into());
        }
    } else {
        return Err("unknown file format: expected a PSD, a Krita (.kra) or an OpenRaster (.ora) file".into());
    };
    if doc.width == 0 || doc.height == 0 {
        return Err("the canvas is empty".into());
    }
    for l in &doc.layers {
        if l.rgba.len() != l.width as usize * l.height as usize * 4 {
            return Err(format!("layer `{}` has inconsistent pixel data", l.name));
        }
    }
    Ok(doc)
}

/// Crops the layer to the bounding box of its non-transparent pixels. Returns
/// `false` when the layer is fully transparent (nothing left).
pub fn trim(layer: &mut Layer) -> bool {
    let (w, h) = (layer.width as usize, layer.height as usize);
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            if layer.rgba[(y * w + x) * 4 + 3] != 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x1 <= x0 || y1 <= y0 {
        return false;
    }
    if (x0, y0, x1, y1) == (0, 0, w, h) {
        return true;
    }
    let nw = x1 - x0;
    let mut out = Vec::with_capacity(nw * (y1 - y0) * 4);
    for y in y0..y1 {
        out.extend_from_slice(&layer.rgba[(y * w + x0) * 4..(y * w + x1) * 4]);
    }
    layer.rgba = out;
    layer.x += x0 as i32;
    layer.y += y0 as i32;
    layer.width = nw as u32;
    layer.height = (y1 - y0) as u32;
    true
}

/// Encodes RGBA8 pixels as a PNG file.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let img = image::RgbaImage::from_raw(width, height, rgba.to_vec()).ok_or("bad layer pixels")?;
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(out)
}

/// Largest layer we accept, so a hostile file cannot exhaust memory.
pub(crate) const MAX_PIXELS: u64 = 64 * 1024 * 1024;

pub(crate) fn pixels(w: u32, h: u32) -> Result<usize, String> {
    let n = u64::from(w) * u64::from(h);
    if n > MAX_PIXELS { Err(format!("a layer of {w}x{h} pixels is too large")) } else { Ok(n as usize) }
}

#[cfg(test)]
mod tests;
