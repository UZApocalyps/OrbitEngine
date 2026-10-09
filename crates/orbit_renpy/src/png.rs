//! A tiny PNG writer for the solid backgrounds Ren'Py names without a file
//! (`scene black`): stored (uncompressed) deflate, no image dependency.

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &d in data {
        a = (a + u32::from(d)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// An RGB PNG of `width` x `height` filled with `rgb` (small sizes only:
/// one stored deflate block, at most 65 535 bytes of pixels).
pub fn solid_png(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    solid(width, height, &rgb)
}

/// [`solid_png`] with an alpha channel (`Solid("#ff000080")`).
pub fn solid_png_rgba(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
    solid(width, height, &rgba)
}

fn solid(width: u32, height: u32, px: &[u8]) -> Vec<u8> {
    // Each row: filter byte 0 (none), then the pixels.
    let row: Vec<u8> = std::iter::once(0)
        .chain(px.iter().copied().cycle().take(width as usize * px.len()))
        .collect();
    let raw = row.repeat(height as usize);
    assert!(raw.len() <= 65_535, "solid_png is for small images");
    let mut z = vec![0x78, 0x01, 0x01];
    let len = raw.len() as u16;
    z.extend_from_slice(&len.to_le_bytes());
    z.extend_from_slice(&(!len).to_le_bytes());
    z.extend_from_slice(&raw);
    z.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    let color_type = if px.len() == 4 { 6 } else { 2 };
    ihdr.extend_from_slice(&[8, color_type, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Pixel size of a PNG or JPEG file, read from its header (no decoding).
pub fn image_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.get(12..16) == Some(b"IHDR") {
        let be = |i: usize| -> Option<u32> {
            Some(u32::from_be_bytes(bytes.get(i..i + 4)?.try_into().ok()?))
        };
        return Some((be(16)?, be(20)?));
    }
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    // JPEG: walk the segments up to a start-of-frame marker.
    let mut i = 2;
    while i + 9 < bytes.len() {
        if bytes[i] != 0xff {
            i += 1;
            continue;
        }
        let marker = bytes[i + 1];
        let len = usize::from(u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]));
        let sof = matches!(marker, 0xc0..=0xcf) && !matches!(marker, 0xc4 | 0xc8 | 0xcc);
        if sof {
            let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]);
            let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]);
            return Some((u32::from(w), u32::from(h)));
        }
        i += 2 + len;
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn known_crc() {
        assert_eq!(super::crc32(b"IEND"), 0xae42_6082);
        assert!(super::solid_png(2, 2, [0, 0, 0]).starts_with(b"\x89PNG"));
    }

    #[test]
    fn sizes() {
        assert_eq!(
            super::image_size(&super::solid_png(7, 3, [1, 2, 3])),
            Some((7, 3))
        );
        assert_eq!(
            super::image_size(&super::solid_png_rgba(2, 5, [1, 2, 3, 4])),
            Some((2, 5))
        );
        let jpeg = [
            0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0, 0xff, 0xc0, 0, 11, 8, 0, 20, 0, 30, 3, 0, 0, 0,
        ];
        assert_eq!(super::image_size(&jpeg), Some((30, 20)));
        assert_eq!(super::image_size(b"nope"), None);
    }
}
