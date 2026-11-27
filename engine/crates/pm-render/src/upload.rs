//! Reused staging memory for uploads recorded into a command encoder.
//!
//! wgpu-core creates a fresh Metal buffer (a kernel call) for every
//! `Queue::write_buffer` and `Queue::write_texture`. A [`Belt`] keeps
//! mapped staging chunks instead: bytes are copied into the open chunk,
//! the copy is recorded into the caller's encoder, and [`Belt::close`]
//! hands the chunk back mapped once the GPU has finished with that
//! submission. A chunk whose encoder is dropped unsubmitted is released.

use std::sync::mpsc;

/// Copy offsets into a chunk; buffer-to-texture copies need 256.
const ALIGN: u64 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as u64;
/// The smallest chunk; one frame's glyph edges usually fit in one.
const MIN_CHUNK: u64 = 256 * 1024;
/// Mapped chunks kept for reuse beyond this are released.
const FREE_BYTES: u64 = 32 * 1024 * 1024;

struct Chunk {
    buffer: wgpu::Buffer,
    used: u64,
}

/// Staging chunks reused across submissions.
pub struct Belt {
    open: Option<Chunk>,
    free: Vec<Chunk>,
    back: mpsc::Receiver<Chunk>,
    send: mpsc::Sender<Chunk>,
    /// Chunks created (not reused) since the belt was made.
    pub created: u64,
}

impl Default for Belt {
    fn default() -> Self {
        let (send, back) = mpsc::channel();
        Self {
            open: None,
            free: Vec::new(),
            back,
            send,
            created: 0,
        }
    }
}

impl Belt {
    /// Copies `bytes` into staging memory and returns where they are:
    /// the buffer and offset to record a copy from. The copy must go into
    /// the encoder later passed to [`Belt::close`].
    pub fn stage(&mut self, device: &wgpu::Device, bytes: &[u8]) -> (wgpu::Buffer, u64) {
        let size = (bytes.len() as u64)
            .max(1)
            .next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
        let fits = |c: &Chunk| c.used.next_multiple_of(ALIGN) + size <= c.buffer.size();
        if !self.open.as_ref().is_some_and(fits) {
            // A chunk that cannot take these bytes waits in `free` until
            // the next close hands every open chunk to the GPU.
            if let Some(full) = self.open.take() {
                self.free.push(full);
            }
            self.receive();
            let at = self.free.iter().position(|c| c.used == 0 && fits(c));
            self.open = Some(match at {
                Some(i) => self.free.swap_remove(i),
                None => {
                    self.created += 1;
                    Chunk {
                        buffer: device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("pm upload belt"),
                            size: size.next_power_of_two().max(MIN_CHUNK),
                            usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC,
                            mapped_at_creation: true,
                        }),
                        used: 0,
                    }
                }
            });
        }
        let chunk = self.open.as_mut().expect("open chunk");
        let offset = chunk.used.next_multiple_of(ALIGN);
        chunk
            .buffer
            .slice(offset..offset + size)
            .get_mapped_range_mut()
            .expect("an open belt chunk is mapped")
            .slice(..bytes.len())
            .copy_from_slice(bytes);
        chunk.used = offset + size;
        (chunk.buffer.clone(), offset)
    }

    /// Unmaps every chunk written since the last close; each comes back
    /// mapped for reuse once `encoder`'s submission has run.
    pub fn close(&mut self, encoder: &wgpu::CommandEncoder) {
        let (written, unused): (Vec<Chunk>, Vec<Chunk>) = self
            .open
            .take()
            .into_iter()
            .chain(std::mem::take(&mut self.free))
            .partition(|c| c.used > 0);
        self.free = unused;
        for chunk in written {
            chunk.buffer.unmap();
            let send = self.send.clone();
            let buffer = chunk.buffer.clone();
            encoder.map_buffer_on_submit(&buffer, wgpu::MapMode::Write, .., move |mapped| {
                // A failed map is never reused: it would still be unmapped.
                if mapped.is_ok() {
                    let _ = send.send(Chunk {
                        buffer: chunk.buffer,
                        used: 0,
                    });
                }
            });
        }
    }

    fn receive(&mut self) {
        while let Ok(chunk) = self.back.try_recv() {
            self.free.push(chunk);
        }
        // Keep the largest chunks within the bound.
        let mut kept = 0;
        self.free
            .sort_by_key(|c| std::cmp::Reverse(c.buffer.size()));
        self.free.retain(|c| {
            kept += c.buffer.size();
            c.used > 0 || kept <= FREE_BYTES
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_bytes_reach_the_target_and_chunks_are_reused_after_submit() {
        let gpu = pm_gpu::Gpu::new("upload belt test").unwrap();
        let (device, queue) = (&gpu.device, &gpu.queue);
        let target = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 1024,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut belt = Belt::default();
        for round in 0..4u8 {
            let mut encoder = device.create_command_encoder(&Default::default());
            for part in 0..4u8 {
                let bytes = [round * 16 + part; 256];
                let (buffer, offset) = belt.stage(device, &bytes);
                encoder.copy_buffer_to_buffer(&buffer, offset, &target, u64::from(part) * 256, 256);
            }
            belt.close(&encoder);
            queue.submit([encoder.finish()]);
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            let slice = target.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            let read = slice.get_mapped_range().unwrap();
            for part in 0..4usize {
                assert!(
                    read[part * 256..(part + 1) * 256]
                        .iter()
                        .all(|&b| b == round * 16 + part as u8)
                );
            }
            drop(read);
            target.unmap();
        }
        assert_eq!(belt.created, 1, "one chunk serves every round");
    }

    #[test]
    fn a_dropped_encoder_releases_its_chunk() {
        let gpu = pm_gpu::Gpu::new("upload belt test").unwrap();
        let device = &gpu.device;
        let mut belt = Belt::default();
        let encoder = device.create_command_encoder(&Default::default());
        let _ = belt.stage(device, &[1; 64]);
        belt.close(&encoder);
        drop(encoder);
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let encoder = device.create_command_encoder(&Default::default());
        let _ = belt.stage(device, &[2; 64]);
        belt.close(&encoder);
        drop(encoder);
        assert_eq!(
            belt.created, 2,
            "the dropped chunk is never handed back unmapped"
        );
    }
}
