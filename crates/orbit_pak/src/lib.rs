//! OrbitEngine game files.
//!
//! Two formats, both without any I/O so that the same code runs natively and
//! in the browser (see ADR 0010):
//!
//! * [`container`]: the **pak**, a versioned container of named entries
//!   (index first, then the data, each entry optionally LZ4 compressed and
//!   protected by a CRC-32). The index is small and self-contained, so a web
//!   player can fetch it alone, then ask for single entries with HTTP range
//!   requests;
//! * [`game`]: the **game pack**, a compiled story: the `orbit_core::Story`
//!   and its translations in postcard, plus the game identity (name, id,
//!   version, default language). It is one entry (`game.pack`) of the pak.
//!
//! [`trailer`] describes the 16 bytes appended after a pak that is glued to
//! the end of a player executable.

pub mod container;
mod crc;
pub mod game;
mod legacy;
pub mod trailer;

pub use container::{
    Codec, Compression, Entry, HEADER_LEN, Header, PakError, PakIndex, PakWriter, WriteStats,
};
pub use crc::crc32;
pub use game::{ENGINE_PREFIX, GAME_PACK_ENTRY, GameInfo, GamePack, ICON_ENTRY, PackError};
