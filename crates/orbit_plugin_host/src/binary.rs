//! First check of a plugin file, before anything loads it: is it a dynamic
//! library for this platform and this processor? Only the headers are read.

use std::io::Read;
use std::path::Path;

/// Extension of a plugin library on this platform (no dot).
pub const fn library_extension() -> &'static str {
    if cfg!(target_os = "windows") {
        "dll"
    } else if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    }
}

/// The executable formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Windows PE.
    Pe,
    /// Linux and others.
    Elf,
    /// macOS (thin).
    MachO,
    /// macOS universal binary (several architectures).
    MachOFat,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Self::Pe => "Windows (PE)",
            Self::Elf => "Linux (ELF)",
            Self::MachO | Self::MachOFat => "macOS (Mach-O)",
        }
    }

    /// The format libraries have on this platform.
    pub const fn native() -> Self {
        if cfg!(target_os = "windows") {
            Self::Pe
        } else if cfg!(target_os = "macos") {
            Self::MachO
        } else {
            Self::Elf
        }
    }
}

/// What the headers say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub format: Format,
    /// `x86_64`, `aarch64`, `x86`... (`None` for a universal binary).
    pub arch: Option<String>,
    /// A shared library (not an executable or object file).
    pub dynamic: bool,
}

/// Why a file is not a loadable plugin library here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BinaryError {
    Unreadable(String),
    /// Not an executable format at all.
    NotALibrary,
    /// A library of another operating system.
    WrongPlatform { found: &'static str, expected: &'static str },
    /// A library for another processor.
    WrongArch { found: String, expected: &'static str },
    /// An executable, not a shared library.
    NotDynamic,
}

impl std::fmt::Display for BinaryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable(e) => write!(f, "the file cannot be read: {e}"),
            Self::NotALibrary => f.write_str("the file is not a dynamic library"),
            Self::WrongPlatform { found, expected } => {
                write!(f, "library for {found}, this editor runs on {expected}")
            }
            Self::WrongArch { found, expected } => {
                write!(f, "library for {found} processors, this editor runs on {expected}")
            }
            Self::NotDynamic => f.write_str("the file is a program, not a dynamic library"),
        }
    }
}

/// The processor this editor runs on, named like [`Header::arch`].
pub const fn native_arch() -> &'static str {
    if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else if cfg!(target_arch = "x86") {
        "x86"
    } else {
        "other"
    }
}

fn u16_le(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Reads the headers in `bytes` (the start of the file: 4 KiB is plenty).
pub fn parse_header(bytes: &[u8]) -> Option<Header> {
    if bytes.starts_with(b"MZ") {
        let pe = u32_le(bytes, 0x3c)? as usize;
        if bytes.get(pe..pe + 4)? != b"PE\0\0" {
            return None;
        }
        let machine = u16_le(bytes, pe + 4)?;
        let characteristics = u16_le(bytes, pe + 22)?;
        let arch = match machine {
            0x8664 => "x86_64".to_owned(),
            0xaa64 => "aarch64".to_owned(),
            0x014c => "x86".to_owned(),
            other => format!("PE machine {other:#06x}"),
        };
        return Some(Header {
            format: Format::Pe,
            arch: Some(arch),
            dynamic: characteristics & 0x2000 != 0,
        });
    }
    if bytes.starts_with(b"\x7fELF") {
        let little = *bytes.get(5)? == 1;
        let read16 = |at| {
            let b: [u8; 2] = bytes.get(at..at + 2)?.try_into().ok()?;
            Some(if little { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
        };
        let kind = read16(16)?;
        let machine = read16(18)?;
        let arch = match machine {
            62 => "x86_64".to_owned(),
            183 => "aarch64".to_owned(),
            3 => "x86".to_owned(),
            other => format!("ELF machine {other}"),
        };
        return Some(Header {
            format: Format::Elf,
            arch: Some(arch),
            // ET_DYN (shared objects; also PIE executables, which cannot
            // export the plugin symbols: the loader reports those).
            dynamic: kind == 3,
        });
    }
    match bytes.get(0..4)? {
        [0xcf, 0xfa, 0xed, 0xfe] => {
            let cpu = u32_le(bytes, 4)?;
            let filetype = u32_le(bytes, 12)?;
            let arch = match cpu {
                0x0100_0007 => "x86_64".to_owned(),
                0x0100_000c => "aarch64".to_owned(),
                other => format!("Mach-O cpu {other:#x}"),
            };
            Some(Header {
                format: Format::MachO,
                arch: Some(arch),
                dynamic: filetype == 6 || filetype == 8,
            })
        }
        [0xca, 0xfe, 0xba, 0xbe] => Some(Header {
            format: Format::MachOFat,
            arch: None,
            dynamic: true,
        }),
        _ => None,
    }
}

/// Checks headers against this platform and processor.
pub fn check_header(bytes: &[u8]) -> Result<Header, BinaryError> {
    let header = parse_header(bytes).ok_or(BinaryError::NotALibrary)?;
    let native = Format::native();
    let same_format = header.format == native
        || (native == Format::MachO && header.format == Format::MachOFat);
    if !same_format {
        return Err(BinaryError::WrongPlatform {
            found: header.format.name(),
            expected: native.name(),
        });
    }
    if let Some(arch) = &header.arch
        && arch != native_arch()
    {
        return Err(BinaryError::WrongArch {
            found: arch.clone(),
            expected: native_arch(),
        });
    }
    if !header.dynamic {
        return Err(BinaryError::NotDynamic);
    }
    Ok(header)
}

/// [`check_header`] on the start of a file.
pub fn check_library_file(path: &Path) -> Result<Header, BinaryError> {
    let mut file = std::fs::File::open(path).map_err(|e| BinaryError::Unreadable(e.to_string()))?;
    let mut buf = Vec::with_capacity(4096);
    file.by_ref()
        .take(4096)
        .read_to_end(&mut buf)
        .map_err(|e| BinaryError::Unreadable(e.to_string()))?;
    check_header(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pe(machine: u16, characteristics: u16) -> Vec<u8> {
        let mut b = vec![0u8; 512];
        b[0..2].copy_from_slice(b"MZ");
        b[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        b[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
        b[0x80 + 22..0x80 + 24].copy_from_slice(&characteristics.to_le_bytes());
        b
    }

    fn elf(kind: u16, machine: u16) -> Vec<u8> {
        let mut b = vec![0u8; 64];
        b[0..4].copy_from_slice(b"\x7fELF");
        b[4] = 2;
        b[5] = 1;
        b[16..18].copy_from_slice(&kind.to_le_bytes());
        b[18..20].copy_from_slice(&machine.to_le_bytes());
        b
    }

    fn macho(cpu: u32, filetype: u32) -> Vec<u8> {
        let mut b = vec![0u8; 32];
        b[0..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
        b[4..8].copy_from_slice(&cpu.to_le_bytes());
        b[12..16].copy_from_slice(&filetype.to_le_bytes());
        b
    }

    #[test]
    fn headers_are_recognised() {
        let h = parse_header(&pe(0x8664, 0x2022)).unwrap();
        assert_eq!((h.format, h.arch.as_deref(), h.dynamic), (Format::Pe, Some("x86_64"), true));
        let h = parse_header(&pe(0xaa64, 0x0022)).unwrap();
        assert_eq!((h.arch.as_deref(), h.dynamic), (Some("aarch64"), false));
        let h = parse_header(&elf(3, 62)).unwrap();
        assert_eq!((h.format, h.arch.as_deref(), h.dynamic), (Format::Elf, Some("x86_64"), true));
        let h = parse_header(&macho(0x0100_000c, 6)).unwrap();
        assert_eq!((h.format, h.arch.as_deref(), h.dynamic), (Format::MachO, Some("aarch64"), true));
        assert!(parse_header(b"PK\x03\x04 not a library").is_none());
        assert!(parse_header(b"").is_none());
        assert!(parse_header(b"MZ").is_none());
    }

    #[test]
    fn only_a_native_library_passes() {
        let native = match Format::native() {
            Format::Pe => pe(
                match native_arch() {
                    "aarch64" => 0xaa64,
                    _ => 0x8664,
                },
                0x2022,
            ),
            Format::Elf => elf(3, if native_arch() == "aarch64" { 183 } else { 62 }),
            _ => macho(if native_arch() == "aarch64" { 0x0100_000c } else { 0x0100_0007 }, 6),
        };
        if native_arch() == "x86_64" || native_arch() == "aarch64" {
            assert!(check_header(&native).is_ok());
        }
        assert_eq!(check_header(b"hello"), Err(BinaryError::NotALibrary));
        // Another platform's library.
        let foreign = if Format::native() == Format::Elf { pe(0x8664, 0x2022) } else { elf(3, 62) };
        assert!(matches!(check_header(&foreign), Err(BinaryError::WrongPlatform { .. })));
        // Another processor.
        if Format::native() == Format::Pe && native_arch() == "x86_64" {
            assert!(matches!(check_header(&pe(0x014c, 0x2022)), Err(BinaryError::WrongArch { .. })));
            // An executable.
            assert_eq!(check_header(&pe(0x8664, 0x0022)), Err(BinaryError::NotDynamic));
        }
    }
}
