//! Photoshop `.psd` / `.psb`: the layer records and their channel data only
//! (the merged image is never read). 8 and 16-bit RGB, raw and RLE (PackBits)
//! channels. Zip-compressed channels, masks, blend modes, adjustment and text
//! layers are skipped or ignored with a warning.

use crate::{Group, Layer, LayeredDoc};

struct Rd<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Rd<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.p.checked_add(n).filter(|e| *e <= self.d.len()).ok_or("truncated PSD file")?;
        let s = &self.d[self.p..end];
        self.p = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn i16(&mut self) -> Result<i16, String> {
        Ok(self.u16()? as i16)
    }
    fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn i32(&mut self) -> Result<i32, String> {
        Ok(self.u32()? as i32)
    }
    fn u64(&mut self) -> Result<u64, String> {
        let b = self.take(8)?;
        Ok(u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }
    /// A length that is 8 bytes in PSB files.
    fn len(&mut self, big: bool) -> Result<usize, String> {
        let v = if big { self.u64()? } else { u64::from(self.u32()?) };
        usize::try_from(v).map_err(|_| "PSD length too large".to_owned())
    }
    fn skip(&mut self, n: usize) -> Result<(), String> {
        self.take(n).map(|_| ())
    }
}

struct Channel {
    id: i16,
    len: usize,
}

struct Record {
    top: i32,
    left: i32,
    bottom: i32,
    right: i32,
    channels: Vec<Channel>,
    blend: [u8; 4],
    opacity: u8,
    visible: bool,
    name: String,
    /// `lsct` section type: 1 / 2 folder start, 3 divider.
    section: u32,
    /// An additional-info key that marks a non-raster layer, if any.
    special: Option<String>,
}

/// PSB uses 8 byte lengths for these additional-info keys.
fn big_key(key: &[u8]) -> bool {
    matches!(key, b"LMsk" | b"Lr16" | b"Lr32" | b"Layr" | b"Mt16" | b"Mt32" | b"Mtrn" | b"Alph" | b"FMsk" | b"lnk2" | b"FEid" | b"FXid" | b"PxSD")
}

fn utf16(data: &[u8]) -> Option<String> {
    let n = u32::from_be_bytes(data.get(0..4)?.try_into().ok()?) as usize;
    let units: Vec<u16> = data.get(4..4 + n * 2)?.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
    Some(String::from_utf16_lossy(&units))
}

pub fn parse(data: &[u8]) -> Result<LayeredDoc, String> {
    let mut r = Rd { d: data, p: 4 };
    let version = r.u16()?;
    if version != 1 && version != 2 {
        return Err("unsupported PSD version".into());
    }
    let big = version == 2;
    r.skip(6)?;
    let _channels = r.u16()?;
    let height = r.u32()?;
    let width = r.u32()?;
    let depth = r.u16()?;
    let mode = r.u16()?;
    if mode != 3 {
        return Err(format!("unsupported colour mode {mode}: convert the file to RGB"));
    }
    if depth != 8 && depth != 16 {
        return Err(format!("unsupported bit depth {depth}: convert the file to 8 or 16 bits per channel"));
    }
    let n = r.u32()? as usize;
    r.skip(n)?; // colour mode data
    let n = r.u32()? as usize;
    r.skip(n)?; // image resources
    let lm_len = r.len(big)?;
    let lm_start = r.p;
    let lm_end = lm_start.checked_add(lm_len).filter(|e| *e <= data.len()).ok_or("truncated PSD file")?;
    let mut doc = LayeredDoc { width, height, ..LayeredDoc::default() };
    if lm_len == 0 {
        return Ok(doc);
    }
    let li_len = r.len(big)?;
    let li_start = r.p;
    if li_len > 0 {
        let body = data.get(li_start..li_start + li_len).ok_or("truncated PSD file")?;
        layers(body, big, depth, &mut doc)?;
    } else {
        // 16 / 32-bit files keep their layers in an `Lr16` / `Lr32` block
        // after the global mask info.
        r.p = li_start;
        let gm = r.u32()? as usize;
        r.skip(gm)?;
        while r.p + 12 <= lm_end {
            let sig = r.take(4)?;
            if sig != b"8BIM" && sig != b"8B64" {
                break;
            }
            let key = r.take(4)?;
            let len = r.len(big_key(key) && big)?;
            let block = r.take(len)?;
            if key == b"Lr16" || key == b"Lr32" {
                layers(block, big, depth, &mut doc)?;
                break;
            }
            if len % 4 != 0 {
                r.skip(4 - len % 4).ok();
            }
        }
    }
    Ok(doc)
}

/// Parses a layer info body (count, records, channel data).
fn layers(body: &[u8], big: bool, depth: u16, doc: &mut LayeredDoc) -> Result<(), String> {
    let mut r = Rd { d: body, p: 0 };
    let count = i32::from(r.i16()?).unsigned_abs() as usize;
    let mut recs = Vec::with_capacity(count);
    for _ in 0..count {
        recs.push(record(&mut r, big)?);
    }
    // Groups: records run bottom to top, a divider (type 3) opens a group
    // that its folder record (type 1 / 2, carrying the name) closes.
    let mut open: Vec<usize> = Vec::new();
    for rec in recs {
        let chain = open.clone();
        let (cw, ch) = ((rec.right - rec.left).max(0) as u32, (rec.bottom - rec.top).max(0) as u32);
        match rec.section {
            3 => {
                doc.groups.push(Group { name: "group".into(), parent: open.last().copied(), visible: true });
                open.push(doc.groups.len() - 1);
                skip_channels(&mut r, &rec)?;
                continue;
            }
            1 | 2 => {
                if let Some(g) = open.pop() {
                    doc.groups[g].name = rec.name.clone();
                    doc.groups[g].visible = rec.visible;
                }
                skip_channels(&mut r, &rec)?;
                continue;
            }
            _ => {}
        }
        if let Some(k) = &rec.special {
            doc.warnings.push(format!("layer `{}` skipped: `{k}` layers (text, adjustment, smart object...) are not raster", rec.name));
            skip_channels(&mut r, &rec)?;
            continue;
        }
        if cw == 0 || ch == 0 {
            doc.warnings.push(format!("layer `{}` has no pixels: skipped", rec.name));
            skip_channels(&mut r, &rec)?;
            continue;
        }
        let px = crate::pixels(cw, ch)?;
        if &rec.blend != b"norm" && &rec.blend != b"pass" {
            doc.warnings.push(format!("layer `{}`: blend mode `{}` is ignored", rec.name, String::from_utf8_lossy(&rec.blend)));
        }
        let mut rgba = vec![255u8; px * 4];
        let (mut has_color, mut masked) = ([false; 3], false);
        for c in &rec.channels {
            let chunk = r.take(c.len)?;
            let plane = match channel(chunk, cw as usize, ch as usize, depth, big) {
                Ok(p) => p,
                Err(e) => {
                    doc.warnings.push(format!("layer `{}`: channel {} ignored ({e})", rec.name, c.id));
                    continue;
                }
            };
            let slot = match c.id {
                0..=2 => {
                    has_color[c.id as usize] = true;
                    c.id as usize
                }
                -1 => 3,
                _ => {
                    masked = true;
                    continue;
                }
            };
            for (i, v) in plane.iter().enumerate() {
                rgba[i * 4 + slot] = *v;
            }
        }
        if masked {
            doc.warnings.push(format!("layer `{}`: its mask is ignored", rec.name));
        }
        if !has_color.iter().all(|c| *c) {
            doc.warnings.push(format!("layer `{}` skipped: it has no RGB pixels", rec.name));
            continue;
        }
        doc.layers.push(Layer {
            name: rec.name,
            groups: chain,
            visible: rec.visible,
            opacity: f32::from(rec.opacity) / 255.0,
            x: rec.left,
            y: rec.top,
            width: cw,
            height: ch,
            rgba,
        });
    }
    Ok(())
}

fn skip_channels(r: &mut Rd, rec: &Record) -> Result<(), String> {
    for c in &rec.channels {
        r.skip(c.len)?;
    }
    Ok(())
}

fn record(r: &mut Rd, big: bool) -> Result<Record, String> {
    let (top, left, bottom, right) = (r.i32()?, r.i32()?, r.i32()?, r.i32()?);
    let nch = r.u16()? as usize;
    let mut channels = Vec::with_capacity(nch);
    for _ in 0..nch {
        let id = r.i16()?;
        let len = r.len(big)?;
        channels.push(Channel { id, len });
    }
    if r.take(4)? != b"8BIM" {
        return Err("bad PSD layer record".into());
    }
    let blend: [u8; 4] = r.take(4)?.try_into().map_err(|_| "bad blend")?;
    let opacity = r.u8()?;
    let _clip = r.u8()?;
    let flags = r.u8()?;
    r.skip(1)?;
    let extra_len = r.u32()? as usize;
    let extra = r.take(extra_len)?;
    let mut e = Rd { d: extra, p: 0 };
    let mask = e.u32()? as usize;
    e.skip(mask)?;
    let ranges = e.u32()? as usize;
    e.skip(ranges)?;
    let nlen = e.u8()? as usize;
    let name_bytes = e.take(nlen)?;
    // The Pascal string is padded to a multiple of 4 bytes (length byte included).
    let pad = (4 - (1 + nlen) % 4) % 4;
    e.skip(pad)?;
    let mut name = name_bytes.iter().map(|b| *b as char).collect::<String>();
    let mut section = 0;
    let mut special = None;
    while e.p + 12 <= extra.len() {
        let sig = e.take(4)?;
        if sig != b"8BIM" && sig != b"8B64" {
            break;
        }
        let key: [u8; 4] = e.take(4)?.try_into().map_err(|_| "bad key")?;
        let len = e.len(big && big_key(&key))?;
        let block = e.take(len)?;
        if len % 2 == 1 {
            e.skip(1).ok();
        }
        match &key {
            b"lsct" | b"lsdk" => section = u32::from_be_bytes(block.get(0..4).and_then(|b| b.try_into().ok()).unwrap_or([0; 4])),
            b"luni" => {
                if let Some(n) = utf16(block) {
                    name = n;
                }
            }
            b"TySh" | b"tySh" | b"SoLd" | b"SoLE" | b"PlLd" => {
                special.get_or_insert_with(|| String::from_utf8_lossy(&key).into_owned());
            }
            k if matches!(k, b"brit" | b"levl" | b"curv" | b"expA" | b"vibA" | b"hue2" | b"blnc" | b"blwh" | b"phfl" | b"mixr" | b"clrL" | b"thrs" | b"nvrt" | b"post" | b"selc" | b"grdm" | b"CgEd" | b"SoCo" | b"GdFl" | b"PtFl") => {
                special.get_or_insert_with(|| String::from_utf8_lossy(&key).into_owned());
            }
            _ => {}
        }
    }
    if name.is_empty() {
        name = "layer".into();
    }
    Ok(Record { top, left, bottom, right, channels, blend, opacity, visible: flags & 2 == 0, name, section, special })
}

/// One channel (compression tag + data) to 8-bit samples, `w * h` of them.
fn channel(chunk: &[u8], w: usize, h: usize, depth: u16, big: bool) -> Result<Vec<u8>, String> {
    let bps = (depth / 8) as usize;
    let mut r = Rd { d: chunk, p: 0 };
    let comp = r.u16()?;
    let row_bytes = w * bps;
    let mut raw = Vec::with_capacity(row_bytes * h);
    match comp {
        0 => raw.extend_from_slice(r.take(row_bytes * h)?),
        1 => {
            let mut counts = Vec::with_capacity(h);
            for _ in 0..h {
                counts.push(if big { r.u32()? as usize } else { r.u16()? as usize });
            }
            for c in counts {
                let line = r.take(c)?;
                let before = raw.len();
                packbits(line, row_bytes, &mut raw)?;
                if raw.len() - before != row_bytes {
                    return Err("corrupt RLE data".into());
                }
            }
        }
        _ => return Err("ZIP-compressed channels are not supported".into()),
    }
    if bps == 2 {
        // Keep the high byte of each big-endian sample.
        return Ok(raw.chunks_exact(2).map(|c| c[0]).collect());
    }
    Ok(raw)
}

fn packbits(src: &[u8], want: usize, out: &mut Vec<u8>) -> Result<(), String> {
    let start = out.len();
    let mut i = 0;
    while i < src.len() && out.len() - start < want {
        let n = src[i] as i8;
        i += 1;
        if n >= 0 {
            let c = n as usize + 1;
            out.extend_from_slice(src.get(i..i + c).ok_or("corrupt RLE data")?);
            i += c;
        } else if n != -128 {
            let c = 1 - n as isize;
            let v = *src.get(i).ok_or("corrupt RLE data")?;
            i += 1;
            out.extend(std::iter::repeat_n(v, c as usize));
        }
    }
    out.truncate(start + want.min(out.len() - start));
    Ok(())
}
