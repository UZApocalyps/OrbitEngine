//! GPU pass timings with wgpu timestamp queries (TL-04).
//!
//! Only created when the profiler is compiled in (`orbit_profile::COMPILED`)
//! and the adapter offers `TIMESTAMP_QUERY` (never on WebGL2): otherwise
//! [`crate::Gfx::timestamp_writes`] returns `None` and passes are recorded
//! exactly as before. Nothing ever blocks: the queries of a frame are
//! resolved and copied after its passes, mapped asynchronously and read on a
//! later frame; frames recorded while a read is in flight are not timed.

use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// Most timed passes per frame (two queries each).
pub const MAX_TIMED_PASSES: u32 = 16;

const IDLE: u8 = 0;
const PENDING: u8 = 1;
const MAPPED: u8 = 2;
const FAILED: u8 = 3;

pub(crate) struct GpuTimer {
    query_set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: Arc<wgpu::Buffer>,
    /// Nanoseconds per timestamp tick.
    period: f64,
    /// Passes handed out this frame.
    next: AtomicU32,
    labels: Mutex<Vec<&'static str>>,
    /// Labels of the frame being read back.
    in_flight: Mutex<Vec<&'static str>>,
    state: Arc<AtomicU8>,
}

impl GpuTimer {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let count = MAX_TIMED_PASSES * 2;
        let size = u64::from(count) * u64::from(wgpu::QUERY_SIZE);
        Self {
            query_set: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("orbit gpu timer"),
                ty: wgpu::QueryType::Timestamp,
                count,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("orbit gpu timer resolve"),
                size,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            readback: Arc::new(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("orbit gpu timer readback"),
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })),
            period: f64::from(queue.get_timestamp_period()),
            next: AtomicU32::new(0),
            labels: Mutex::new(Vec::new()),
            in_flight: Mutex::new(Vec::new()),
            state: Arc::new(AtomicU8::new(IDLE)),
        }
    }

    /// Query indices for one more pass of this frame, if there is room and
    /// no read-back is using the buffers.
    pub(crate) fn writes(
        &self,
        label: &'static str,
    ) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        if !orbit_profile::is_enabled() || self.state.load(Ordering::Acquire) != IDLE {
            return None;
        }
        let pass = self.next.fetch_add(1, Ordering::Relaxed);
        if pass >= MAX_TIMED_PASSES {
            return None;
        }
        if let Ok(mut labels) = self.labels.lock() {
            labels.push(label);
        }
        Some(wgpu::RenderPassTimestampWrites {
            query_set: &self.query_set,
            beginning_of_pass_write_index: Some(pass * 2),
            end_of_pass_write_index: Some(pass * 2 + 1),
        })
    }

    /// Ends the frame: reports a finished read-back to the profiler, then
    /// resolves this frame's queries (if any) and starts reading them.
    pub(crate) fn end_frame(&self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let _ = device.poll(wgpu::PollType::Poll);
        match self.state.load(Ordering::Acquire) {
            MAPPED => {
                let labels = self
                    .in_flight
                    .lock()
                    .map(|mut l| std::mem::take(&mut *l))
                    .unwrap_or_default();
                let passes = match self.readback.slice(..).get_mapped_range() {
                    Ok(data) => {
                        let ticks: Vec<u64> = data
                            .chunks_exact(8)
                            .map(|c| u64::from_le_bytes(c.try_into().unwrap_or([0; 8])))
                            .collect();
                        durations(&labels, &ticks, self.period)
                    }
                    Err(_) => Vec::new(),
                };
                self.readback.unmap();
                self.state.store(IDLE, Ordering::Release);
                orbit_profile::report_gpu(passes);
            }
            FAILED => self.state.store(IDLE, Ordering::Release),
            _ => {}
        }
        let used = self.next.swap(0, Ordering::Relaxed).min(MAX_TIMED_PASSES);
        let labels = self
            .labels
            .lock()
            .map(|mut l| std::mem::take(&mut *l))
            .unwrap_or_default();
        if used == 0 || self.state.load(Ordering::Acquire) != IDLE {
            return;
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("orbit gpu timer resolve"),
        });
        encoder.resolve_query_set(&self.query_set, 0..used * 2, &self.resolve, 0);
        // The whole buffer is copied and mapped (only `used` pairs are read).
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.readback, 0, self.resolve.size());
        queue.submit(Some(encoder.finish()));
        if let Ok(mut in_flight) = self.in_flight.lock() {
            *in_flight = labels;
        }
        self.state.store(PENDING, Ordering::Release);
        let state = self.state.clone();
        self.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| {
                state.store(if r.is_ok() { MAPPED } else { FAILED }, Ordering::Release);
            });
    }
}

/// Pass durations in milliseconds from begin/end tick pairs (passes whose
/// timestamps went backwards, e.g. after a GPU clock reset, are dropped).
pub(crate) fn durations(
    labels: &[&'static str],
    ticks: &[u64],
    period_ns: f64,
) -> Vec<(&'static str, f64)> {
    labels
        .iter()
        .enumerate()
        .filter_map(|(i, &label)| {
            let (begin, end) = (*ticks.get(i * 2)?, *ticks.get(i * 2 + 1)?);
            (end >= begin).then(|| (label, (end - begin) as f64 * period_ns / 1.0e6))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_convert_ticks_to_milliseconds() {
        let out = durations(&["a", "b", "c"], &[100, 1_100, 50, 40, 0, 2_000_000], 1.0);
        assert_eq!(out, vec![("a", 0.001), ("c", 2.0)]);
        // A label without its pair of queries is ignored.
        assert!(durations(&["a"], &[1], 1.0).is_empty());
    }
}
