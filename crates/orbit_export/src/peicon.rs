//! Replaces the icon of a Windows executable (PE) without any external tool.
//!
//! The player template embeds the OrbitEngine icon (`RT_GROUP_ICON` /
//! `RT_ICON` resources, see `crates/orbit_player/build.rs`). To give a game its
//! own icon, the whole resource tree is read, the icon entries are replaced,
//! and the rebuilt tree is stored in a **new section appended at the end of
//! the file** (the old `.rsrc` is usually followed by `.reloc`, so it cannot
//! grow in place). The old section is renamed and left unused.
//!
//! The icon sizes are PNG-compressed entries (Windows Vista and later).

use std::io::Cursor;

const RT_ICON: u32 = 3;
const RT_GROUP_ICON: u32 = 14;
/// Standard icon sizes, smallest first.
pub(crate) const SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];

fn rd16(b: &[u8], o: usize) -> Result<u16, String> {
    b.get(o..o + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or_else(|| "truncated executable".to_owned())
}
fn rd32(b: &[u8], o: usize) -> Result<u32, String> {
    b.get(o..o + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| "truncated executable".to_owned())
}
fn put32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
fn align(v: usize, a: usize) -> usize {
    v.div_ceil(a.max(1)) * a.max(1)
}

struct Res {
    ty: u32,
    name: u32,
    lang: u32,
    codepage: u32,
    data: Vec<u8>,
}

/// (virtual size, virtual address, raw size, raw pointer) of a section.
type Section = (usize, usize, usize, usize);

fn rva_to_off(sections: &[Section], rva: usize) -> Result<usize, String> {
    sections
        .iter()
        .find(|(vs, va, rs, _)| rva >= *va && rva < va + (*vs).max(*rs))
        .map(|(_, va, _, ptr)| ptr + rva - va)
        .ok_or_else(|| "resource outside of any section".to_owned())
}

/// Entries `(id, offset)` of the resource directory at file offset `dir`.
fn dir_entries(exe: &[u8], dir: usize) -> Result<Vec<(u32, u32)>, String> {
    let n = rd16(exe, dir + 12)? as usize + rd16(exe, dir + 14)? as usize;
    (0..n)
        .map(|i| Ok((rd32(exe, dir + 16 + i * 8)?, rd32(exe, dir + 20 + i * 8)?)))
        .collect()
}

fn read_resources(exe: &[u8], sections: &[Section], rsrc_rva: usize) -> Result<Vec<Res>, String> {
    let base = rva_to_off(sections, rsrc_rva)?;
    let sub = |off: u32| base + (off & 0x7fff_ffff) as usize;
    let mut res = Vec::new();
    for (ty, t_off) in dir_entries(exe, base)? {
        for (name, n_off) in dir_entries(exe, sub(t_off))? {
            for (lang, l_off) in dir_entries(exe, sub(n_off))? {
                if (ty | name | lang | l_off) & 0x8000_0000 != 0 {
                    return Err("named resources are not supported".into());
                }
                let de = base + l_off as usize;
                let off = rva_to_off(sections, rd32(exe, de)? as usize)?;
                let size = rd32(exe, de + 4)? as usize;
                let data = exe.get(off..off + size).ok_or("truncated resource")?;
                res.push(Res {
                    ty,
                    name,
                    lang,
                    codepage: rd32(exe, de + 8)?,
                    data: data.to_vec(),
                });
            }
        }
    }
    Ok(res)
}

/// Sets the icon of `exe` from the PNG `png` (any size, resized to every
/// standard size). Returns the patched file.
pub(crate) fn set_icon(exe: &[u8], png: &[u8]) -> Result<Vec<u8>, String> {
    let (icons, group) = build_icon_resources(png)?;
    let mut exe = exe.to_vec();

    if exe.get(..2) != Some(b"MZ") {
        return Err("not a Windows executable".into());
    }
    let pe = rd32(&exe, 0x3c)? as usize;
    if exe.get(pe..pe + 4) != Some(b"PE\0\0") {
        return Err("not a PE file".into());
    }
    let nsec = rd16(&exe, pe + 6)? as usize;
    let opt_size = rd16(&exe, pe + 20)? as usize;
    let opt = pe + 24;
    let dd = match rd16(&exe, opt)? {
        0x10b => opt + 96,
        0x20b => opt + 112,
        _ => return Err("unknown PE format".into()),
    };
    let sec_align = rd32(&exe, opt + 32)? as usize;
    let file_align = rd32(&exe, opt + 36)? as usize;
    let headers_size = rd32(&exe, opt + 60)? as usize;
    let table = opt + opt_size;
    let sections: Vec<Section> = (0..nsec)
        .map(|i| {
            let s = table + i * 40;
            Ok((
                rd32(&exe, s + 8)? as usize,
                rd32(&exe, s + 12)? as usize,
                rd32(&exe, s + 16)? as usize,
                rd32(&exe, s + 20)? as usize,
            ))
        })
        .collect::<Result<_, String>>()?;
    if table + (nsec + 1) * 40 > headers_size {
        return Err("no room for a new section header".into());
    }
    let rsrc_rva = rd32(&exe, dd + 16)? as usize;

    let mut res = if rsrc_rva != 0 {
        read_resources(&exe, &sections, rsrc_rva)?
    } else {
        Vec::new()
    };
    let lang = res
        .iter()
        .find(|r| r.ty == RT_GROUP_ICON)
        .map_or(0x0409, |r| r.lang);
    res.retain(|r| r.ty != RT_ICON && r.ty != RT_GROUP_ICON);
    for (i, data) in icons.into_iter().enumerate() {
        res.push(Res { ty: RT_ICON, name: i as u32 + 1, lang, codepage: 0, data });
    }
    res.push(Res { ty: RT_GROUP_ICON, name: 1, lang, codepage: 0, data: group });
    res.sort_by_key(|r| (r.ty, r.name, r.lang));

    // New section after everything, in the image and in the file.
    let last_va_end = sections
        .iter()
        .map(|(vs, va, rs, _)| va + (*vs).max(*rs))
        .max()
        .unwrap_or(0);
    let new_va = align(last_va_end, sec_align);
    let blob = build_tree(&res, new_va as u32);
    let new_ptr = align(exe.len(), file_align);
    let raw_size = align(blob.len(), file_align);
    exe.resize(new_ptr, 0);
    exe.extend_from_slice(&blob);
    exe.resize(new_ptr + raw_size, 0);

    // The old `.rsrc` is renamed so only one section has that name.
    for i in 0..nsec {
        let s = table + i * 40;
        if exe[s..s + 5] == *b".rsrc" {
            exe[s..s + 8].copy_from_slice(b".oldrsrc");
        }
    }
    let s = table + nsec * 40;
    exe[s..s + 40].fill(0);
    exe[s..s + 5].copy_from_slice(b".rsrc");
    put32(&mut exe, s + 8, blob.len() as u32);
    put32(&mut exe, s + 12, new_va as u32);
    put32(&mut exe, s + 16, raw_size as u32);
    put32(&mut exe, s + 20, new_ptr as u32);
    put32(&mut exe, s + 36, 0x4000_0040); // initialized data, readable
    exe[pe + 6..pe + 8].copy_from_slice(&(nsec as u16 + 1).to_le_bytes());
    put32(&mut exe, dd + 16, new_va as u32);
    put32(&mut exe, dd + 20, blob.len() as u32);
    put32(&mut exe, opt + 56, align(new_va + blob.len(), sec_align) as u32);
    put32(&mut exe, opt + 64, 0); // checksum: not verified for user programs
    Ok(exe)
}

/// PNG-compressed `RT_ICON` payloads and the `RT_GROUP_ICON` directory.
fn build_icon_resources(png: &[u8]) -> Result<(Vec<Vec<u8>>, Vec<u8>), String> {
    let src = image::load_from_memory(png).map_err(|e| e.to_string())?;
    let mut icons = Vec::new();
    let mut group = vec![0, 0, 1, 0];
    group.extend_from_slice(&(SIZES.len() as u16).to_le_bytes());
    for (i, size) in SIZES.iter().enumerate() {
        let img = src.resize_to_fill(*size, *size, image::imageops::FilterType::Lanczos3);
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        let data = out.into_inner();
        let dim = if *size >= 256 { 0 } else { *size as u8 };
        group.extend_from_slice(&[dim, dim, 0, 0]);
        group.extend_from_slice(&1u16.to_le_bytes()); // planes
        group.extend_from_slice(&32u16.to_le_bytes()); // bit count
        group.extend_from_slice(&(data.len() as u32).to_le_bytes());
        group.extend_from_slice(&(i as u16 + 1).to_le_bytes());
        icons.push(data);
    }
    Ok((icons, group))
}

/// Serializes the resource tree (type / name / language) as a section whose
/// first byte sits at RVA `va`. `res` must be sorted by (type, name, lang).
fn build_tree(res: &[Res], va: u32) -> Vec<u8> {
    type Names = Vec<(u32, Vec<usize>)>;
    let mut types: Vec<(u32, Names)> = Vec::new();
    for (i, r) in res.iter().enumerate() {
        if types.last().is_none_or(|t| t.0 != r.ty) {
            types.push((r.ty, Vec::new()));
        }
        let names = &mut types.last_mut().unwrap().1;
        if names.last().is_none_or(|n| n.0 != r.name) {
            names.push((r.name, Vec::new()));
        }
        names.last_mut().unwrap().1.push(i);
    }
    let dir_size = |n: usize| 16 + 8 * n;
    let mut dirs = dir_size(types.len());
    for (_, names) in &types {
        dirs += dir_size(names.len());
        for (_, langs) in names {
            dirs += dir_size(langs.len());
        }
    }
    let entries_at = dirs;
    let mut data_at = entries_at + 16 * res.len();
    let mut out = vec![0u8; data_at];
    let mut offs = Vec::new();
    for r in res {
        data_at = align(data_at, 4);
        offs.push(data_at);
        data_at += r.data.len();
    }
    let set_count = |out: &mut Vec<u8>, at: usize, n: usize| {
        out[at + 14..at + 16].copy_from_slice(&(n as u16).to_le_bytes());
    };
    set_count(&mut out, 0, types.len());
    let mut cur = dir_size(types.len());
    for (ti, (ty, names)) in types.iter().enumerate() {
        let e = 16 + ti * 8;
        put32(&mut out, e, *ty);
        put32(&mut out, e + 4, 0x8000_0000 | cur as u32);
        let name_dir = cur;
        cur += dir_size(names.len());
        set_count(&mut out, name_dir, names.len());
        for (ni, (name, langs)) in names.iter().enumerate() {
            let e = name_dir + 16 + ni * 8;
            put32(&mut out, e, *name);
            put32(&mut out, e + 4, 0x8000_0000 | cur as u32);
            let lang_dir = cur;
            cur += dir_size(langs.len());
            set_count(&mut out, lang_dir, langs.len());
            for (li, &ri) in langs.iter().enumerate() {
                let e = lang_dir + 16 + li * 8;
                put32(&mut out, e, res[ri].lang);
                put32(&mut out, e + 4, (entries_at + 16 * ri) as u32);
                let d = entries_at + 16 * ri;
                put32(&mut out, d, va + offs[ri] as u32);
                put32(&mut out, d + 4, res[ri].data.len() as u32);
                put32(&mut out, d + 8, res[ri].codepage);
            }
        }
    }
    for (r, off) in res.iter().zip(&offs) {
        out.resize(*off, 0);
        out.extend_from_slice(&r.data);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ICON: &[u8] = include_bytes!("../../../assets/brand/icon-256.png");

    /// `RT_ICON` payloads read back from the resource tree of `exe`.
    fn read_icons(exe: &[u8]) -> Vec<Vec<u8>> {
        let pe = rd32(exe, 0x3c).unwrap() as usize;
        let nsec = rd16(exe, pe + 6).unwrap() as usize;
        let opt = pe + 24;
        let dd = if rd16(exe, opt).unwrap() == 0x10b { opt + 96 } else { opt + 112 };
        let table = opt + rd16(exe, pe + 20).unwrap() as usize;
        let sections: Vec<Section> = (0..nsec)
            .map(|i| {
                let s = table + i * 40;
                (
                    rd32(exe, s + 8).unwrap() as usize,
                    rd32(exe, s + 12).unwrap() as usize,
                    rd32(exe, s + 16).unwrap() as usize,
                    rd32(exe, s + 20).unwrap() as usize,
                )
            })
            .collect();
        let rva = rd32(exe, dd + 16).unwrap() as usize;
        let res = read_resources(exe, &sections, rva).unwrap();
        res.into_iter().filter(|r| r.ty == RT_ICON).map(|r| r.data).collect()
    }

    #[test]
    fn icon_is_replaced_in_a_real_pe() {
        // The test executable itself is a PE on Windows (with its resources
        // if any); other hosts have nothing to patch.
        if !cfg!(windows) {
            return;
        }
        let exe = std::fs::read(std::env::current_exe().unwrap()).unwrap();
        let patched = set_icon(&exe, ICON).unwrap();
        let (expect, group) = build_icon_resources(ICON).unwrap();
        assert_eq!(read_icons(&patched), expect);
        // Patching twice replaces again instead of accumulating.
        let twice = set_icon(&patched, ICON).unwrap();
        assert_eq!(read_icons(&twice), expect);
        assert_eq!(group.len(), 6 + 14 * SIZES.len());
    }

    #[test]
    fn non_pe_input_is_refused() {
        assert!(set_icon(b"\x7fELF....", ICON).is_err());
    }
}
