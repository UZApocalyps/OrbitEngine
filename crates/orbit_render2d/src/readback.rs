//! Asynchronous read-back of a render target into CPU memory.
//!
//! `map_async` is the only way to read GPU memory that works in every wgpu
//! backend including WebGL2, where blocking is impossible: the copy is
//! submitted by [`crate::Renderer2d::start_readback`] and the host polls
//! [`crate::Renderer2d::poll_readback`] once per frame until the pixels are
//! available. Nothing here ever blocks.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use orbit_gfx::wgpu;

/// Rows of a texture-to-buffer copy start at multiples of this many bytes.
const ROW_ALIGNMENT: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

const PENDING: u8 = 0;
const MAPPED: u8 = 1;
const FAILED: u8 = 2;

/// A copy of a render target on its way to the CPU.
pub struct Readback {
    pub(crate) buffer: wgpu::Buffer,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) padded_row: u32,
    pub(crate) state: Arc<AtomicU8>,
    pub(crate) finished: bool,
}

/// Progress of a [`Readback`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadbackStatus {
    /// The GPU has not finished yet; poll again next frame.
    Pending,
    /// Tightly packed RGBA8 pixels (sRGB-encoded, top row first).
    Ready {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    /// The copy failed (device lost...) or the result was already taken.
    Failed,
}

/// Bytes per row of the staging buffer for a texture `width` pixels wide.
pub(crate) fn padded_bytes_per_row(width: u32) -> u32 {
    (width * 4).div_ceil(ROW_ALIGNMENT) * ROW_ALIGNMENT
}

/// Removes the per-row padding of a mapped staging buffer.
pub(crate) fn unpad_rows(data: &[u8], width: u32, height: u32, padded_row: u32) -> Vec<u8> {
    let row = width as usize * 4;
    let mut out = Vec::with_capacity(row * height as usize);
    for y in 0..height as usize {
        let start = y * padded_row as usize;
        out.extend_from_slice(&data[start..start + row]);
    }
    out
}

impl Readback {
    pub(crate) fn new(device: &wgpu::Device, width: u32, height: u32) -> (Self, Arc<AtomicU8>) {
        let padded_row = padded_bytes_per_row(width);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("orbit2d readback"),
            size: u64::from(padded_row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let state = Arc::new(AtomicU8::new(PENDING));
        (
            Self {
                buffer,
                width,
                height,
                padded_row,
                state: state.clone(),
                finished: false,
            },
            state,
        )
    }

    /// Records the map callback result (called from `map_async`).
    pub(crate) fn complete(state: &AtomicU8, ok: bool) {
        state.store(if ok { MAPPED } else { FAILED }, Ordering::Release);
    }

    /// Takes the pixels if the buffer is mapped.
    pub(crate) fn take(&mut self) -> ReadbackStatus {
        if self.finished {
            return ReadbackStatus::Failed;
        }
        match self.state.load(Ordering::Acquire) {
            MAPPED => {
                self.finished = true;
                let rgba = match self.buffer.slice(..).get_mapped_range() {
                    Ok(view) => Some(unpad_rows(&view, self.width, self.height, self.padded_row)),
                    Err(_) => None,
                };
                self.buffer.unmap();
                match rgba {
                    Some(rgba) => ReadbackStatus::Ready {
                        width: self.width,
                        height: self.height,
                        rgba,
                    },
                    None => ReadbackStatus::Failed,
                }
            }
            FAILED => {
                self.finished = true;
                ReadbackStatus::Failed
            }
            _ => ReadbackStatus::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_padded_to_256_bytes() {
        assert_eq!(padded_bytes_per_row(320), 1280); // already aligned
        assert_eq!(padded_bytes_per_row(100), 512); // 400 -> 512
        assert_eq!(padded_bytes_per_row(1), 256);
    }

    #[test]
    fn padding_is_removed_row_by_row() {
        // 2x2 pixels, rows padded to 256 bytes.
        let mut data = vec![0u8; 512];
        data[..8].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        data[256..264].copy_from_slice(&[9, 10, 11, 12, 13, 14, 15, 16]);
        let out = unpad_rows(&data, 2, 2, 256);
        assert_eq!(out, (1..=16).collect::<Vec<u8>>());
    }
}
