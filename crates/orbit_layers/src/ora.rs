//! OpenRaster (`.ora`): a zip with `stack.xml` (layers top to bottom, nested
//! `<stack>` groups) and one PNG per layer.

use std::io::{Read, Seek};

use zip::ZipArchive;

use crate::xml;
use crate::{Group, Layer, LayeredDoc};

pub(crate) fn read_entry<R: Read + Seek>(zip: &mut ZipArchive<R>, name: &str) -> Result<Vec<u8>, String> {
    let mut f = zip.by_name(name).map_err(|_| format!("`{name}` is missing from the archive"))?;
    if f.size() > 512 * 1024 * 1024 {
        return Err(format!("`{name}` is too large"));
    }
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).map_err(|e| format!("cannot read `{name}`: {e}"))?;
    Ok(buf)
}

pub(crate) fn decode_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?.to_rgba8();
    crate::pixels(img.width(), img.height())?;
    Ok((img.width(), img.height(), img.into_raw()))
}

pub fn parse<R: Read + Seek>(zip: &mut ZipArchive<R>) -> Result<LayeredDoc, String> {
    let text = String::from_utf8_lossy(&read_entry(zip, "stack.xml")?).into_owned();
    let mut doc = LayeredDoc::default();
    // Open stacks: `None` for the root one, else the group index.
    let mut open: Vec<Option<usize>> = Vec::new();
    for t in xml::tags(&text) {
        match (t.name.as_str(), t.open, t.close) {
            ("image", true, _) => {
                doc.width = t.attr("w").and_then(|v| v.parse().ok()).unwrap_or(0);
                doc.height = t.attr("h").and_then(|v| v.parse().ok()).unwrap_or(0);
            }
            ("stack", true, closed) => {
                let hidden = t.attr("visibility") == Some("hidden");
                if open.is_empty() {
                    open.push(None);
                } else {
                    doc.groups.push(Group {
                        name: t.attr("name").unwrap_or("group").to_owned(),
                        parent: open.iter().rev().find_map(|g| *g),
                        visible: !hidden,
                    });
                    open.push(Some(doc.groups.len() - 1));
                }
                if closed {
                    open.pop();
                }
            }
            ("stack", false, _) => {
                open.pop();
            }
            ("layer", true, _) => {
                let Some(src) = t.attr("src") else { continue };
                let name = t.attr("name").filter(|n| !n.is_empty()).unwrap_or(src).to_owned();
                let bytes = match read_entry(zip, src) {
                    Ok(b) => b,
                    Err(e) => {
                        doc.warnings.push(format!("layer `{name}` skipped: {e}"));
                        continue;
                    }
                };
                let (width, height, rgba) = match decode_png(&bytes) {
                    Ok(v) => v,
                    Err(e) => {
                        doc.warnings.push(format!("layer `{name}` skipped: cannot decode its PNG ({e})"));
                        continue;
                    }
                };
                if let Some(op) = t.attr("composite-op")
                    && op != "svg:src-over"
                {
                    doc.warnings.push(format!("layer `{name}`: blend mode `{op}` is ignored"));
                }
                doc.layers.push(Layer {
                    name,
                    groups: open.iter().filter_map(|g| *g).collect(),
                    visible: t.attr("visibility") != Some("hidden"),
                    opacity: t.attr("opacity").and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0).clamp(0.0, 1.0),
                    x: t.attr("x").and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0) as i32,
                    y: t.attr("y").and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0) as i32,
                    width,
                    height,
                    rgba,
                });
            }
            _ => {}
        }
    }
    // The stack lists the top layer first.
    doc.layers.reverse();
    Ok(doc)
}
