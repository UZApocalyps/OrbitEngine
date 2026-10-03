//! KTX2 textures (M6-13, ADR 0014 section 3 and addendum M6-13).
//!
//! Pure Rust and wasm friendly: the container is parsed by the `ktx2` crate
//! (Apache-2.0), Zstandard supercompression is undone by `ruzstd` (MIT) and
//! zlib by `flate2` (miniz_oxide backend).
//!
//! What is supported:
//! * [`info`]: size, format family and mip count of **any** KTX2 file
//!   (export validation, editor library);
//! * [`decode_rgba8`]: level 0 of an uncompressed 8-bit RGBA / RGB texture
//!   (UNORM or sRGB), with no, Zstandard or zlib supercompression, into a
//!   [`TextureAsset`] that every renderer path (WebGL2 included) uploads.
//!
//! Not supported, refused with a clear message: GPU block-compressed payloads
//! (BCn, ETC2, ASTC: the renderer keeps CPU RGBA8 textures for now) and Basis
//! Universal (ETC1S / UASTC, which need a transcoder: the reference one is
//! C++, see the ADR addendum).

use std::io::Read;

use ::ktx2::{ColorModel, Reader};
/// Re-exported for [`encode_simple`] and callers matching formats.
pub use ::ktx2::{Format, SupercompressionScheme};

use crate::TextureAsset;

/// The family of a KTX2 payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ktx2Kind {
    /// 8-bit RGBA or RGB, decodable by [`decode_rgba8`].
    Rgba8,
    /// Another uncompressed format (16-bit, float, single channel...).
    OtherUncompressed,
    /// BC1-7 / ETC2 / EAC / ASTC blocks.
    BlockCompressed,
    /// Basis Universal ETC1S (BasisLZ) or UASTC: must be transcoded.
    Basis,
}

/// What a KTX2 file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ktx2Info {
    pub width: u32,
    pub height: u32,
    pub levels: u32,
    pub kind: Ktx2Kind,
    pub srgb: bool,
    pub cubemap: bool,
}

/// `true` when `bytes` start with the KTX2 identifier.
pub fn is_ktx2(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xAB, b'K', b'T', b'X', b' ', b'2', b'0', 0xBB, b'\r', b'\n', 0x1A, b'\n'])
}

fn reader(bytes: &[u8]) -> Result<Reader<&[u8]>, String> {
    Reader::new(bytes).map_err(|e| format!("invalid KTX2: {e:?}"))
}

fn kind_of(reader: &Reader<&[u8]>) -> (Ktx2Kind, bool) {
    let header = reader.header();
    match header.format {
        None => {
            let basis = header.supercompression_scheme == Some(SupercompressionScheme::BasisLZ)
                || matches!(
                    reader.basic_dfd().map(|d| d.color_model),
                    Some(Some(ColorModel::ETC1S | ColorModel::UASTC))
                );
            let kind = if basis {
                Ktx2Kind::Basis
            } else {
                Ktx2Kind::OtherUncompressed
            };
            let srgb = reader
                .transfer_function()
                .is_some_and(|t| t == ::ktx2::TransferFunction::SRGB);
            (kind, srgb)
        }
        Some(f) => match f {
            Format::R8G8B8A8_UNORM | Format::R8G8B8_UNORM => (Ktx2Kind::Rgba8, false),
            Format::R8G8B8A8_SRGB | Format::R8G8B8_SRGB => (Ktx2Kind::Rgba8, true),
            f if (Format::BC1_RGB_UNORM_BLOCK.value()..=Format::ASTC_12x12_SRGB_BLOCK.value())
                .contains(&f.value()) =>
            {
                // Vulkan numbers BC (131-146), ETC2/EAC (147-156) and ASTC
                // (157-184) contiguously; sRGB variants are the even/odd twin.
                let srgb = format!("{f:?}").contains("SRGB");
                (Ktx2Kind::BlockCompressed, srgb)
            }
            _ => (Ktx2Kind::OtherUncompressed, false),
        },
    }
}

/// Size, family and mip count of a KTX2 file.
pub fn info(bytes: &[u8]) -> Result<Ktx2Info, String> {
    let r = reader(bytes)?;
    let h = r.header();
    let (kind, srgb) = kind_of(&r);
    Ok(Ktx2Info {
        width: h.pixel_width,
        height: h.pixel_height.max(1),
        levels: h.level_count.max(1),
        kind,
        srgb,
        cubemap: h.face_count == 6,
    })
}

/// Level 0 of an uncompressed 8-bit RGBA / RGB KTX2 texture, as RGBA8.
pub fn decode_rgba8(name: &str, bytes: &[u8]) -> Result<TextureAsset, String> {
    let r = reader(bytes)?;
    let h = r.header();
    let (kind, _) = kind_of(&r);
    match kind {
        Ktx2Kind::Rgba8 => {}
        Ktx2Kind::Basis => {
            return Err(
                "Basis Universal KTX2 (ETC1S / UASTC) is not supported yet: export the texture as PNG, or as uncompressed KTX2 (`toktx --t2 --zcmp`)"
                    .into(),
            );
        }
        Ktx2Kind::BlockCompressed => {
            return Err(
                "GPU block-compressed KTX2 (BCn / ETC2 / ASTC) is not supported yet: use PNG or uncompressed KTX2"
                    .into(),
            );
        }
        Ktx2Kind::OtherUncompressed => {
            return Err(format!(
                "KTX2 format {:?} is not supported (8-bit RGBA or RGB only)",
                h.format
            ));
        }
    }
    if h.pixel_depth > 1 || h.layer_count > 1 || h.face_count != 1 {
        return Err("KTX2 arrays, cubemaps and 3D textures are not supported here".into());
    }
    let (width, height) = (h.pixel_width, h.pixel_height.max(1));
    if width == 0 || width > 16384 || height > 16384 {
        return Err(format!("unsupported KTX2 size {width}x{height}"));
    }
    let level = r.levels().next().ok_or("KTX2 without image data")?;
    let raw = match h.supercompression_scheme {
        None => level.data.to_vec(),
        Some(SupercompressionScheme::Zstandard) => {
            let mut out = Vec::with_capacity(level.uncompressed_byte_length as usize);
            ruzstd::decoding::StreamingDecoder::new(level.data)
                .map_err(|e| format!("KTX2 zstd: {e}"))?
                .read_to_end(&mut out)
                .map_err(|e| format!("KTX2 zstd: {e}"))?;
            out
        }
        Some(SupercompressionScheme::ZLIB) => {
            let mut out = Vec::with_capacity(level.uncompressed_byte_length as usize);
            flate2::read::ZlibDecoder::new(level.data)
                .read_to_end(&mut out)
                .map_err(|e| format!("KTX2 zlib: {e}"))?;
            out
        }
        Some(other) => return Err(format!("KTX2 supercompression {other:?} is not supported")),
    };
    let channels = if matches!(h.format, Some(Format::R8G8B8_UNORM | Format::R8G8B8_SRGB)) {
        3
    } else {
        4
    };
    let texels = width as usize * height as usize;
    if raw.len() < texels * channels {
        return Err(format!(
            "KTX2 level 0 holds {} bytes, {} expected",
            raw.len(),
            texels * channels
        ));
    }
    let rgba8 = if channels == 4 {
        raw[..texels * 4].to_vec()
    } else {
        raw[..texels * 3]
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect()
    };
    Ok(TextureAsset {
        name: name.to_owned(),
        width,
        height,
        rgba8,
    })
}

/// Test and tooling helper: a minimal single-level KTX2 file.
#[doc(hidden)]
pub fn encode_simple(
    format: Format,
    width: u32,
    height: u32,
    level0: &[u8],
    supercompression: Option<SupercompressionScheme>,
) -> Vec<u8> {
    use ::ktx2::{Header, Index, LevelIndex, dfd};
    let (basic, type_size) = dfd::Basic::from_format(format).expect("known format");
    let block = dfd::Block::Basic(basic).to_vec();
    let data = match supercompression {
        Some(SupercompressionScheme::Zstandard) => ruzstd::encoding::compress_to_vec(
            level0,
            ruzstd::encoding::CompressionLevel::Fastest,
        ),
        _ => level0.to_vec(),
    };
    let dfd_offset = (Header::LENGTH + LevelIndex::LENGTH) as u32;
    let dfd_len = 4 + block.len() as u32;
    let data_offset = u64::from(dfd_offset + dfd_len).next_multiple_of(16);
    let header = Header {
        format: Some(format),
        type_size,
        pixel_width: width,
        pixel_height: height,
        pixel_depth: 0,
        layer_count: 0,
        face_count: 1,
        level_count: 1,
        supercompression_scheme: supercompression,
        index: Index {
            dfd_byte_offset: dfd_offset,
            dfd_byte_length: dfd_len,
            kvd_byte_offset: 0,
            kvd_byte_length: 0,
            sgd_byte_offset: 0,
            sgd_byte_length: 0,
        },
    };
    let mut out = header.as_bytes().to_vec();
    out.extend_from_slice(
        &LevelIndex {
            byte_offset: data_offset,
            byte_length: data.len() as u64,
            uncompressed_byte_length: level0.len() as u64,
        }
        .as_bytes(),
    );
    out.extend_from_slice(&dfd_len.to_le_bytes());
    out.extend_from_slice(&block);
    out.resize(data_offset as usize, 0);
    out.extend_from_slice(&data);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixels(w: u32, h: u32, channels: usize) -> Vec<u8> {
        (0..w as usize * h as usize * channels)
            .map(|i| (i * 7 % 251) as u8)
            .collect()
    }

    #[test]
    fn uncompressed_and_zstd_rgba8_decode_to_the_same_pixels() {
        let px = pixels(8, 4, 4);
        for sc in [None, Some(SupercompressionScheme::Zstandard)] {
            let file = encode_simple(Format::R8G8B8A8_SRGB, 8, 4, &px, sc);
            assert!(is_ktx2(&file));
            let i = info(&file).unwrap();
            assert_eq!((i.width, i.height, i.levels, i.kind, i.srgb), (8, 4, 1, Ktx2Kind::Rgba8, true));
            let t = decode_rgba8("wall", &file).unwrap();
            assert_eq!((t.name.as_str(), t.width, t.height), ("wall", 8, 4));
            assert_eq!(t.rgba8, px);
        }
    }

    #[test]
    fn rgb8_is_expanded_to_opaque_rgba() {
        let px = pixels(2, 2, 3);
        let file = encode_simple(Format::R8G8B8_UNORM, 2, 2, &px, None);
        let t = decode_rgba8("rgb", &file).unwrap();
        assert_eq!(&t.rgba8[..8], &[px[0], px[1], px[2], 255, px[3], px[4], px[5], 255]);
    }

    #[test]
    fn block_compressed_files_are_described_but_refused() {
        // One 4x4 BC7 block = 16 bytes.
        let file = encode_simple(Format::BC7_SRGB_BLOCK, 4, 4, &[0u8; 16], None);
        let i = info(&file).unwrap();
        assert_eq!((i.kind, i.srgb, i.width), (Ktx2Kind::BlockCompressed, true, 4));
        let err = decode_rgba8("bc7", &file).unwrap_err();
        assert!(err.contains("block-compressed"), "{err}");
        assert!(info(b"not a ktx2 file at all, sorry......").is_err());
        assert!(!is_ktx2(b"\x89PNG"));
    }

    #[test]
    fn a_truncated_level_is_an_error_not_a_panic() {
        let file = encode_simple(Format::R8G8B8A8_UNORM, 4, 4, &[1u8; 8], None);
        assert!(decode_rgba8("short", &file).unwrap_err().contains("expected"));
    }
}
