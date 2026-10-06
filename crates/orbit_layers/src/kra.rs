//! Krita (`.kra`): a zip with `maindoc.xml` (the layer tree) and, per paint
//! layer, `<image name>/layers/<file>` in Krita's tile format (64x64 tiles,
//! LZF compressed, channels stored as planes in B, G, R, A order). Only the
//! 8-bit RGBA colour space is read; other layer kinds and spaces are skipped
//! with a warning.

use std::io::{Read, Seek};

use zip::ZipArchive;

use crate::ora::read_entry;
use crate::xml;
use crate::{Group, Layer, LayeredDoc};

pub fn parse<R: Read + Seek>(zip: &mut ZipArchive<R>) -> Result<LayeredDoc, String> {
    let text = String::from_utf8_lossy(&read_entry(zip, "maindoc.xml")?).into_owned();
    let mut doc = LayeredDoc::default();
    let mut image_name = String::new();
    // One entry per open <layer> element: its group index when it is a group.
    let mut open: Vec<Option<usize>> = Vec::new();
    for t in xml::tags(&text) {
        if t.name == "IMAGE" && t.open {
            doc.width = t.attr("width").and_then(|v| v.parse().ok()).unwrap_or(0);
            doc.height = t.attr("height").and_then(|v| v.parse().ok()).unwrap_or(0);
            image_name = t.attr("name").unwrap_or("").to_owned();
            continue;
        }
        if t.name != "layer" {
            continue;
        }
        if !t.open {
            open.pop();
            continue;
        }
        let name = t.attr("name").unwrap_or("layer").to_owned();
        let kind = t.attr("nodetype").unwrap_or("paintlayer");
        let visible = t.attr("visible").is_none_or(|v| v != "0");
        let groups: Vec<usize> = open.iter().filter_map(|g| *g).collect();
        let mut pushed = None;
        match kind {
            "grouplayer" => {
                doc.groups.push(Group { name: name.clone(), parent: groups.last().copied(), visible });
                pushed = Some(doc.groups.len() - 1);
            }
            "paintlayer" => {
                let opacity = t.attr("opacity").and_then(|v| v.parse::<f32>().ok()).unwrap_or(255.0) / 255.0;
                let (ox, oy) = (
                    t.attr("x").and_then(|v| v.parse::<i32>().ok()).unwrap_or(0),
                    t.attr("y").and_then(|v| v.parse::<i32>().ok()).unwrap_or(0),
                );
                if t.attr("colorspacename").is_some_and(|c| c != "RGBA") {
                    doc.warnings.push(format!(
                        "layer `{name}` skipped: colour space `{}` is not supported (use 8-bit RGB/A)",
                        t.attr("colorspacename").unwrap_or("")
                    ));
                } else if let Some(file) = t.attr("filename") {
                    let entry = format!("{image_name}/layers/{file}");
                    match read_entry(zip, &entry).and_then(|b| tiles(&b)) {
                        Ok(Some((x, y, width, height, rgba))) => doc.layers.push(Layer {
                            name: name.clone(),
                            groups: groups.clone(),
                            visible,
                            opacity: opacity.clamp(0.0, 1.0),
                            x: x + ox,
                            y: y + oy,
                            width,
                            height,
                            rgba,
                        }),
                        Ok(None) => doc.warnings.push(format!("layer `{name}` is empty: skipped")),
                        Err(e) => doc.warnings.push(format!("layer `{name}` skipped: {e}")),
                    }
                }
                if t.attr("compositeop").is_some_and(|c| c != "normal") {
                    doc.warnings.push(format!("layer `{name}`: blend mode `{}` is ignored", t.attr("compositeop").unwrap_or("")));
                }
            }
            other => doc.warnings.push(format!("layer `{name}` skipped: `{other}` is not a raster layer")),
        }
        if !t.close {
            open.push(pushed);
        }
    }
    doc.layers.reverse();
    Ok(doc)
}

/// Decompresses one tile stream into a bounding box and RGBA pixels.
fn tiles(data: &[u8]) -> Result<Option<(i32, i32, u32, u32, Vec<u8>)>, String> {
    let mut pos = 0usize;
    let line = |pos: &mut usize| -> Result<String, String> {
        let rest = data.get(*pos..).ok_or("truncated layer data")?;
        let end = rest.iter().position(|b| *b == b'\n').ok_or("truncated layer data")?;
        *pos += end + 1;
        Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
    };
    let (mut tw, mut th, mut ps) = (64usize, 64usize, 4usize);
    let count;
    loop {
        let l = line(&mut pos)?;
        let mut it = l.split_whitespace();
        match (it.next(), it.next().and_then(|v| v.parse::<usize>().ok())) {
            (Some("TILEWIDTH"), Some(v)) => tw = v,
            (Some("TILEHEIGHT"), Some(v)) => th = v,
            (Some("PIXELSIZE"), Some(v)) => ps = v,
            (Some("DATA"), Some(v)) => {
                count = v;
                break;
            }
            _ => {}
        }
    }
    if ps != 4 || tw == 0 || th == 0 || tw > 1024 || th > 1024 {
        return Err("only 8-bit RGBA tiles are supported".into());
    }
    let plane = tw * th;
    let mut decoded: Vec<(i32, i32, Vec<u8>)> = Vec::new();
    for _ in 0..count {
        let head = line(&mut pos)?;
        let f: Vec<&str> = head.split(',').collect();
        if f.len() != 4 {
            return Err("bad tile header".into());
        }
        let (x, y): (i32, i32) = (f[0].trim().parse().map_err(|_| "bad tile x")?, f[1].trim().parse().map_err(|_| "bad tile y")?);
        let size: usize = f[3].trim().parse().map_err(|_| "bad tile size")?;
        let body = data.get(pos..pos + size).ok_or("truncated tile")?;
        pos += size;
        let raw = match body.first() {
            Some(1) => lzf(&body[1..], plane * 4)?,
            Some(0) => body[1..].to_vec(),
            _ => return Err("unknown tile compression".into()),
        };
        if raw.len() < plane * 4 {
            return Err("short tile".into());
        }
        decoded.push((x, y, raw));
    }
    if decoded.is_empty() {
        return Ok(None);
    }
    let x0 = decoded.iter().map(|t| t.0).min().unwrap_or(0);
    let y0 = decoded.iter().map(|t| t.1).min().unwrap_or(0);
    let x1 = decoded.iter().map(|t| t.0).max().unwrap_or(0) + tw as i32;
    let y1 = decoded.iter().map(|t| t.1).max().unwrap_or(0) + th as i32;
    let (w, h) = ((x1 - x0) as u32, (y1 - y0) as u32);
    let mut rgba = vec![0u8; crate::pixels(w, h)? * 4];
    for (tx, ty, raw) in &decoded {
        for row in 0..th {
            for col in 0..tw {
                let src = row * tw + col;
                let dst = (((ty - y0) as usize + row) * w as usize + (tx - x0) as usize + col) * 4;
                // Planes are B, G, R, A.
                rgba[dst] = raw[2 * plane + src];
                rgba[dst + 1] = raw[plane + src];
                rgba[dst + 2] = raw[src];
                rgba[dst + 3] = raw[3 * plane + src];
            }
        }
    }
    Ok(Some((x0, y0, w, h, rgba)))
}

/// liblzf decompression.
pub(crate) fn lzf(input: &[u8], max: usize) -> Result<Vec<u8>, String> {
    let mut out: Vec<u8> = Vec::with_capacity(max);
    let mut i = 0;
    while i < input.len() {
        let ctrl = input[i] as usize;
        i += 1;
        if ctrl < 32 {
            let n = ctrl + 1;
            out.extend_from_slice(input.get(i..i + n).ok_or("corrupt compressed tile")?);
            i += n;
        } else {
            let mut len = ctrl >> 5;
            if len == 7 {
                len += *input.get(i).ok_or("corrupt compressed tile")? as usize;
                i += 1;
            }
            let low = *input.get(i).ok_or("corrupt compressed tile")? as usize;
            i += 1;
            let back = ((ctrl & 0x1f) << 8) + low + 1;
            if back > out.len() {
                return Err("corrupt compressed tile".into());
            }
            for _ in 0..len + 2 {
                out.push(out[out.len() - back]);
            }
        }
        if out.len() > max {
            return Err("corrupt compressed tile".into());
        }
    }
    Ok(out)
}
