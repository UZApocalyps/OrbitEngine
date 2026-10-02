//! Native exports: the player template with the pak glued to its end,
//! in a zip (Windows) or a tar.gz (Linux).
//!
//! `[player executable][pak][trailer]`: the player finds its game by reading
//! the trailer of its own file (see `orbit_pak::trailer`), so the result is a
//! single executable that can be moved and renamed freely.
//!
//! The icon of the `.exe` file itself (Explorer, the desktop) stays the
//! OrbitEngine one: rewriting the resources of a compiled executable needs
//! unsafe Win32 calls or a resource compiler, neither of which the exporter
//! has. The *window* icon (title bar, task bar while the game runs) is the
//! game's own, read from the pak.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use orbit_pak::trailer;
use orbit_project::{ExportProfile, ExportTarget};

use crate::error::{ExportError, Result};
use crate::pakbuild::{BuiltPak, FONT_LICENSE};
use crate::report::Reporter;

/// A file name made of letters, digits, spaces, `-`, `_` and `.`; anything
/// else becomes `_`.
pub(crate) fn file_safe(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').to_owned();
    if trimmed.is_empty() { "game".to_owned() } else { trimmed }
}

/// Name of the archive: `<id>-<version>-<target>.<ext>`.
pub(crate) fn archive_name(profile: &ExportProfile, target: ExportTarget) -> String {
    let ext = match target {
        ExportTarget::Windows => "zip",
        _ => "tar.gz",
    };
    format!(
        "{}-{}-{}.{ext}",
        profile.game_id,
        file_safe(&profile.version).replace(' ', "_"),
        target.id()
    )
}

const WINDOWS_README: &str = "Double-click the .exe to play. Saves are kept in your user profile \
(%APPDATA%). Nothing needs to be installed.\r\n";

const LINUX_README: &str = "Run the executable to play (chmod +x it if your archive tool dropped the \
permission). Saves are kept under ~/.local/share. Needs a Vulkan or OpenGL capable GPU driver.\n";

pub(crate) struct Written {
    pub output: PathBuf,
    pub files: Vec<PathBuf>,
}

/// Writes the archive of `target` (Windows or Linux) into `out_dir`.
pub(crate) fn export(
    pak: &BuiltPak,
    profile: &ExportProfile,
    target: ExportTarget,
    template: &Path,
    out_dir: &Path,
    rep: &mut Reporter<'_>,
) -> Result<Written> {
    rep.step(0.0, &format!("Reading the {} player template", target.id()));
    let player = std::fs::read(template).map_err(ExportError::io(template))?;
    std::fs::create_dir_all(out_dir).map_err(ExportError::io(out_dir))?;
    let output = out_dir.join(archive_name(profile, target));
    let folder = file_safe(&profile.game_name);
    let trailer = trailer::encode(pak.bytes.len() as u64);
    rep.check_cancelled()?;
    rep.step(0.3, &format!("Writing {}", output.display()));
    match target {
        ExportTarget::Windows => {
            let exe = format!("{folder}/{folder}.exe");
            write_zip(&output, &exe, &player, &pak.bytes, &trailer, &folder)?;
        }
        ExportTarget::Linux => {
            let exe = format!("{folder}/{}", profile.game_id);
            write_tar_gz(&output, &exe, &player, &pak.bytes, &trailer, &folder)?;
        }
        ExportTarget::Web => unreachable!("the web has its own exporter"),
    }
    rep.step(1.0, "Archive written");
    Ok(Written {
        files: vec![output.clone()],
        output,
    })
}

fn package_err(path: &Path, what: impl std::fmt::Display) -> ExportError {
    ExportError::Package(format!("cannot write {}: {what}", path.display()))
}

fn write_zip(
    output: &Path,
    exe_name: &str,
    player: &[u8],
    pak: &[u8],
    trailer: &[u8],
    folder: &str,
) -> Result<()> {
    use zip::CompressionMethod::Deflated;
    use zip::write::SimpleFileOptions;

    let file = File::create(output).map_err(ExportError::io(output))?;
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let exe_options = SimpleFileOptions::default()
        .compression_method(Deflated)
        .unix_permissions(0o755)
        .large_file(true);
    let text_options = SimpleFileOptions::default()
        .compression_method(Deflated)
        .unix_permissions(0o644);
    let io = |e: std::io::Error| package_err(output, e);
    let zip_err = |e: zip::result::ZipError| package_err(output, e);

    zip.start_file(exe_name, exe_options).map_err(zip_err)?;
    zip.write_all(player).map_err(io)?;
    zip.write_all(pak).map_err(io)?;
    zip.write_all(trailer).map_err(io)?;
    zip.start_file(format!("{folder}/README.txt"), text_options)
        .map_err(zip_err)?;
    zip.write_all(WINDOWS_README.as_bytes()).map_err(io)?;
    zip.start_file(format!("{folder}/licenses/Inter-OFL.txt"), text_options)
        .map_err(zip_err)?;
    zip.write_all(FONT_LICENSE.as_bytes()).map_err(io)?;
    let mut inner = zip.finish().map_err(zip_err)?;
    inner.flush().map_err(io)?;
    Ok(())
}

fn write_tar_gz(
    output: &Path,
    exe_name: &str,
    player: &[u8],
    pak: &[u8],
    trailer: &[u8],
    folder: &str,
) -> Result<()> {
    let file = File::create(output).map_err(ExportError::io(output))?;
    let encoder = flate2::write::GzEncoder::new(
        std::io::BufWriter::new(file),
        flate2::Compression::new(6),
    );
    let mut tar = tar::Builder::new(encoder);
    let io = |e: std::io::Error| package_err(output, e);

    let mut header = tar::Header::new_gnu();
    header.set_size((player.len() + pak.len() + trailer.len()) as u64);
    header.set_mode(0o755);
    header.set_mtime(0);
    let body = std::io::Read::chain(
        std::io::Read::chain(player, pak),
        trailer,
    );
    tar.append_data(&mut header, exe_name, body).map_err(io)?;

    for (name, text) in [
        (format!("{folder}/README.txt"), LINUX_README),
        (format!("{folder}/licenses/Inter-OFL.txt"), FONT_LICENSE),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(text.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        tar.append_data(&mut header, name, text.as_bytes()).map_err(io)?;
    }
    let encoder = tar.into_inner().map_err(io)?;
    let mut inner = encoder.finish().map_err(io)?;
    inner.flush().map_err(io)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_file_safe() {
        assert_eq!(file_safe("Mon jeu: v1/2"), "Mon jeu_ v1_2");
        assert_eq!(file_safe("  .. "), "game");
        assert_eq!(file_safe("Caf\u{e9}"), "Caf\u{e9}");
        let p = ExportProfile::new("release", "Mon jeu");
        assert_eq!(archive_name(&p, ExportTarget::Windows), "mon-jeu-1.0.0-windows.zip");
        assert_eq!(archive_name(&p, ExportTarget::Linux), "mon-jeu-1.0.0-linux.tar.gz");
    }
}
