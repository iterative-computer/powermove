//! Texture readback (tests, agent captures, the CPU encoder-input
//! reference, `pm-cli parity`).
//!
//! Reads a 2D texture into tightly packed rows, top-down, converting to f32
//! RGBA. Supported formats: `rgba16float`, `rgba32float`, `rgba8unorm`,
//! `rgba16unorm`, `r32float`, `rg32float`.

#[derive(Debug, thiserror::Error)]
pub enum ReadbackError {
    #[error("readback of {0:?} is not supported")]
    Format(wgpu::TextureFormat),
    #[error("map failed: {0}")]
    Map(String),
}

/// IEEE 754 binary16 to f32 (exact).
pub fn half_to_f32(h: u16) -> f32 {
    let sign = u32::from(h & 0x8000) << 16;
    let exp = u32::from((h >> 10) & 0x1f);
    let frac = u32::from(h & 0x3ff);
    match exp {
        0 => {
            let magnitude = (frac as f32) * f32::from_bits(103 << 23);
            f32::from_bits(sign | magnitude.to_bits())
        }
        31 => f32::from_bits(sign | 0x7f80_0000 | (frac << 13)),
        e => f32::from_bits(sign | ((e + 112) << 23) | (frac << 13)),
    }
}

/// f32 to binary16, rounding to nearest even (for uploads and tests).
pub fn f32_to_half(v: f32) -> u16 {
    let bits = v.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let mant = bits & 0x7f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if mant != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 31 {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = mant | 0x80_0000;
        let shift = (14 - e) as u32;
        let half = 1u32 << (shift - 1);
        let rest = m & ((1 << shift) - 1);
        let mut out = m >> shift;
        if rest > half || (rest == half && out & 1 == 1) {
            out += 1;
        }
        return sign | out as u16;
    }
    let mut out = ((e as u32) << 10) | (mant >> 13);
    let rest = mant & 0x1fff;
    if rest > 0x1000 || (rest == 0x1000 && out & 1 == 1) {
        out += 1;
    }
    sign | out as u16
}

/// Channels per texel and a decoder for one channel of a format.
fn layout(format: wgpu::TextureFormat) -> Result<(usize, usize), ReadbackError> {
    use wgpu::TextureFormat as F;
    Ok(match format {
        F::Rgba16Float => (4, 2),
        F::Rgba32Float => (4, 4),
        F::Rgba8Unorm => (4, 1),
        F::Rgba16Unorm => (4, 2),
        F::R32Float => (1, 4),
        F::Rg32Float => (2, 4),
        other => return Err(ReadbackError::Format(other)),
    })
}

/// Reads `texture` (mip 0) as f32 channels, tightly packed, top-down.
/// Blocks until the copy completes.
pub fn read_f32(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> Result<Vec<f32>, ReadbackError> {
    Readback::default().read_f32(device, queue, texture)
}

/// Decodes mapped rows of `format` (`row` useful bytes of `h` rows spaced
/// `padded` apart) into f32 channels, tightly packed, top-down.
fn decode_f32(
    format: wgpu::TextureFormat,
    data: &[u8],
    row: usize,
    padded: usize,
    h: usize,
    mut out: Vec<f32>,
) -> Vec<f32> {
    let (channels, bytes_per_channel) = layout(format).expect("checked by the caller");
    out.clear();
    out.reserve(row / bytes_per_channel * h);
    debug_assert_eq!(row % (channels * bytes_per_channel), 0);
    for y in 0..h {
        let line = &data[y * padded..y * padded + row];
        match (format, bytes_per_channel) {
            (wgpu::TextureFormat::Rgba16Float, 2) => out.extend(
                line.chunks_exact(2)
                    .map(|c| half_to_f32(u16::from_le_bytes([c[0], c[1]]))),
            ),
            (wgpu::TextureFormat::Rgba16Unorm, 2) => out.extend(
                line.chunks_exact(2)
                    .map(|c| f32::from(u16::from_le_bytes([c[0], c[1]])) / 65535.0),
            ),
            (_, 1) => out.extend(line.iter().map(|&b| f32::from(b) / 255.0)),
            (_, 4) => out.extend(
                line.chunks_exact(4)
                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])),
            ),
            _ => unreachable!(),
        }
    }
    out
}

/// Reads `texture` (mip 0) as raw texel bytes, tightly packed rows.
pub fn read_raw(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> Result<Vec<u8>, ReadbackError> {
    with_mapped_rows(device, queue, texture, |data, row, padded, h| {
        if row == padded {
            return data.to_vec();
        }
        let mut out = Vec::with_capacity(row * h);
        for y in 0..h {
            let start = y * padded;
            out.extend_from_slice(&data[start..start + row]);
        }
        out
    })
}

/// Reads mip 0 while its staging buffer is mapped. `data` contains top-down
/// rows of `row` useful bytes spaced `padded` bytes apart; it is valid only
/// for the duration of `convert`. This lets callers encode directly into
/// their final output without allocating a tightly packed intermediate.
pub fn with_mapped_rows<T>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    convert: impl FnOnce(&[u8], usize, usize, usize) -> T,
) -> Result<T, ReadbackError> {
    Readback::default().with_mapped_rows(device, queue, texture, convert)
}

/// A reusable staging buffer for serial readbacks on one GPU device. It is
/// deliberately device-scoped: a lost device drops the buffer with its owner.
#[derive(Default)]
pub struct Readback {
    buffer: Option<wgpu::Buffer>,
    capacity: u64,
    #[cfg(test)]
    allocations: usize,
}

/// A reserved, unmapped staging destination. Record its copy after the
/// render passes in the same encoder; mapping starts only after submission.
#[derive(Clone)]
pub struct ReadbackCopy {
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    row: usize,
    padded: usize,
    size: u64,
}

impl ReadbackCopy {
    pub fn record(&self, encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture) {
        debug_assert_eq!(
            (texture.width(), texture.height()),
            (self.width, self.height)
        );
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded as u32),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadbackTicket {
    slot: usize,
    generation: u64,
}

enum SlotState {
    Free,
    Reserved,
    Mapping(std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>),
}

struct RingSlot {
    buffer: Option<wgpu::Buffer>,
    capacity: u64,
    generation: u64,
    copy: Option<ReadbackCopy>,
    state: SlotState,
}

impl Default for RingSlot {
    fn default() -> Self {
        Self {
            buffer: None,
            capacity: 0,
            generation: 0,
            copy: None,
            state: SlotState::Free,
        }
    }
}

/// At most two copies may be in flight while the presentation thread draws
/// other views. A mapped buffer never receives GPU writes until unmapped.
pub struct ReadbackRing {
    slots: [RingSlot; 2],
    #[cfg(test)]
    allocations: usize,
}

impl Default for ReadbackRing {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| RingSlot::default()),
            #[cfg(test)]
            allocations: 0,
        }
    }
}

impl ReadbackRing {
    pub fn capacity_bytes(&self) -> u64 {
        self.slots.iter().map(|s| s.capacity).sum()
    }

    pub fn in_flight(&self) -> bool {
        self.slots
            .iter()
            .any(|s| !matches!(&s.state, SlotState::Free))
    }

    /// One unusually large capture uses the ring alone; ordinary captures
    /// may occupy both slots. The byte limit also bounds retained buffers.
    pub fn can_reserve(&self, size: u64, max_cached_bytes: u64) -> bool {
        let free = self
            .slots
            .iter()
            .any(|s| matches!(&s.state, SlotState::Free));
        free && (size <= max_cached_bytes
            && !self.slots.iter().any(|s| {
                !matches!(&s.state, SlotState::Free)
                    && s.copy.as_ref().is_some_and(|c| c.size > max_cached_bytes)
            })
            || !self.in_flight())
    }

    /// Reserve a slot for a fresh texture copy. The caller must either
    /// `start_map` after submitting its encoder or `cancel` on render error.
    pub fn reserve(
        &mut self,
        device: &wgpu::Device,
        texture: &wgpu::Texture,
        max_cached_bytes: u64,
    ) -> Result<Option<(ReadbackTicket, ReadbackCopy)>, ReadbackError> {
        let format = texture.format();
        let bpp = format
            .block_copy_size(None)
            .ok_or(ReadbackError::Format(format))?;
        let width = texture.width();
        let height = texture.height();
        let row = width as usize * bpp as usize;
        let padded = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
        let size = padded as u64 * u64::from(height);
        if !self.can_reserve(size, max_cached_bytes) {
            return Ok(None);
        }
        if size > max_cached_bytes {
            for slot in &mut self.slots {
                debug_assert!(matches!(&slot.state, SlotState::Free));
                slot.buffer = None;
                slot.capacity = 0;
            }
        }
        let index = self
            .slots
            .iter()
            .position(|s| matches!(&s.state, SlotState::Free) && s.capacity >= size)
            .or_else(|| {
                self.slots
                    .iter()
                    .position(|s| matches!(&s.state, SlotState::Free))
            })
            .expect("availability checked");
        let slot = &mut self.slots[index];
        if slot.capacity < size {
            slot.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pm snapshot readback ring"),
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }));
            slot.capacity = size;
            #[cfg(test)]
            {
                self.allocations += 1;
            }
        }
        slot.generation = slot.generation.wrapping_add(1);
        let copy = ReadbackCopy {
            buffer: slot.buffer.as_ref().expect("allocated").clone(),
            width,
            height,
            row,
            padded,
            size,
        };
        slot.copy = Some(copy.clone());
        slot.state = SlotState::Reserved;
        Ok(Some((
            ReadbackTicket {
                slot: index,
                generation: slot.generation,
            },
            copy,
        )))
    }

    fn slot_mut(&mut self, ticket: ReadbackTicket) -> &mut RingSlot {
        let slot = &mut self.slots[ticket.slot];
        assert_eq!(slot.generation, ticket.generation, "stale readback ticket");
        slot
    }

    pub fn cancel(&mut self, ticket: ReadbackTicket, max_cached_bytes: u64) {
        let slot = self.slot_mut(ticket);
        if matches!(&slot.state, SlotState::Mapping(_)) {
            slot.copy.as_ref().expect("mapping copy").buffer.unmap();
        } else {
            assert!(matches!(&slot.state, SlotState::Reserved));
        }
        slot.copy = None;
        slot.state = SlotState::Free;
        if slot.capacity > max_cached_bytes {
            slot.buffer = None;
            slot.capacity = 0;
        }
    }

    pub fn start_map(&mut self, ticket: ReadbackTicket, ready: impl FnOnce() + Send + 'static) {
        let slot = self.slot_mut(ticket);
        assert!(matches!(&slot.state, SlotState::Reserved));
        let copy = slot.copy.as_ref().expect("reserved copy");
        let (tx, rx) = std::sync::mpsc::channel();
        copy.buffer
            .slice(..copy.size)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
                ready();
            });
        slot.state = SlotState::Mapping(rx);
    }

    /// Call when the map callback wakes the owner. `None` means this ticket
    /// is still pending; `Some` consumes and unmaps only this slot.
    pub fn try_with_mapped_rows<T>(
        &mut self,
        ticket: ReadbackTicket,
        convert: impl FnOnce(&[u8], usize, usize, usize) -> T,
        max_cached_bytes: u64,
    ) -> Option<Result<T, ReadbackError>> {
        let slot = self.slot_mut(ticket);
        let SlotState::Mapping(rx) = &slot.state else {
            panic!("readback ticket has not started mapping");
        };
        let status = match rx.try_recv() {
            Ok(status) => status.map_err(|e| ReadbackError::Map(e.to_string())),
            Err(std::sync::mpsc::TryRecvError::Empty) => return None,
            Err(e) => Err(ReadbackError::Map(e.to_string())),
        };
        let copy = slot.copy.take().expect("mapping copy");
        let mapped = status.is_ok();
        let result = status.and_then(|()| {
            let data = copy
                .buffer
                .slice(..copy.size)
                .get_mapped_range()
                .map_err(|e| ReadbackError::Map(format!("{e:?}")))?;
            let out = convert(&data, copy.row, copy.padded, copy.height as usize);
            drop(data);
            Ok(out)
        });
        if mapped {
            copy.buffer.unmap();
        } else {
            // A failed map may leave the buffer's map context unusable.
            // Recreate this slot for the next copy instead of reusing it.
            slot.buffer = None;
            slot.capacity = 0;
        }
        slot.state = SlotState::Free;
        if slot.capacity > max_cached_bytes {
            slot.buffer = None;
            slot.capacity = 0;
        }
        Some(result)
    }
}

impl Readback {
    /// Bytes held by the reusable GPU staging buffer.
    pub fn capacity_bytes(&self) -> u64 {
        self.capacity
    }

    /// Release an oversized staging allocation after its mapping is closed.
    pub fn trim(&mut self, max_bytes: u64) {
        if self.capacity > max_bytes {
            self.buffer = None;
            self.capacity = 0;
        }
    }

    /// [`read_f32`] through this staging buffer: the channels are decoded
    /// straight from the mapped rows (no packed byte copy), and the buffer is
    /// kept for the next read (an export's frames read one after another).
    pub fn read_f32(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
    ) -> Result<Vec<f32>, ReadbackError> {
        self.read_f32_into(device, queue, texture, Vec::new())
    }

    /// [`Self::read_f32`] into `out` (cleared first; its allocation is
    /// reused when it is large enough).
    pub fn read_f32_into(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        out: Vec<f32>,
    ) -> Result<Vec<f32>, ReadbackError> {
        let format = texture.format();
        layout(format)?;
        self.with_mapped_rows(device, queue, texture, |data, row, padded, h| {
            decode_f32(format, data, row, padded, h, out)
        })
    }

    /// Copy this texture into the staging buffer and map it afresh. The
    /// buffer can be reused only after the previous mapping was dropped and
    /// unmapped; this method always completes that sequence synchronously.
    pub fn with_mapped_rows<T>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        convert: impl FnOnce(&[u8], usize, usize, usize) -> T,
    ) -> Result<T, ReadbackError> {
        let format = texture.format();
        let bpp = format
            .block_copy_size(None)
            .ok_or(ReadbackError::Format(format))?;
        let (w, h) = (texture.width(), texture.height());
        let row = (w * bpp) as usize;
        let padded = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
        let size = padded as u64 * u64::from(h);
        if self.capacity < size {
            self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pm readback"),
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }));
            self.capacity = size;
            #[cfg(test)]
            {
                self.allocations += 1;
            }
        }
        let buffer = self.buffer.as_ref().expect("created for this readback");
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("pm readback"),
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded as u32),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        let slice = buffer.slice(..size);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv()
            .map_err(|e| ReadbackError::Map(e.to_string()))?
            .map_err(|e| ReadbackError::Map(e.to_string()))?;
        let data = slice
            .get_mapped_range()
            .map_err(|e| ReadbackError::Map(format!("{e:?}")))?;
        let out = convert(&data, row, padded, h as usize);
        drop(data);
        buffer.unmap();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("readback reuse test"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    fn upload(queue: &wgpu::Queue, texture: &wgpu::Texture, bytes: &[u8]) {
        let row = texture.width() * texture.format().block_copy_size(None).unwrap();
        queue.write_texture(
            texture.as_image_copy(),
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(texture.height()),
            },
            wgpu::Extent3d {
                width: texture.width(),
                height: texture.height(),
                depth_or_array_layers: 1,
            },
        );
    }

    #[test]
    fn serial_readback_reuses_staging_across_fresh_copies_formats_and_growth() {
        let gpu = crate::Gpu::new("readback reuse test").unwrap();
        let mut readback = Readback::default();
        let rgba8 = source(&gpu.device, wgpu::TextureFormat::Rgba8Unorm, 2, 1);
        for (index, values) in [[1u8, 2, 3, 4, 5, 6, 7, 8], [8, 7, 6, 5, 4, 3, 2, 1]]
            .iter()
            .enumerate()
        {
            upload(&gpu.queue, &rgba8, values);
            let got = readback
                .with_mapped_rows(&gpu.device, &gpu.queue, &rgba8, |data, row, _, _| {
                    data[..row].to_vec()
                })
                .unwrap();
            assert_eq!(
                got.as_slice(),
                values.as_slice(),
                "copy {index} must show the new pixels"
            );
            assert_eq!(readback.allocations, 1);
        }
        let half = source(&gpu.device, wgpu::TextureFormat::Rgba16Float, 1, 1);
        let half_bytes = [0, 0x3c, 0, 0, 0, 0, 0, 0x3c];
        upload(&gpu.queue, &half, &half_bytes);
        let got = readback
            .with_mapped_rows(&gpu.device, &gpu.queue, &half, |data, row, _, _| {
                data[..row].to_vec()
            })
            .unwrap();
        assert_eq!(got.as_slice(), half_bytes.as_slice());
        assert_eq!(readback.allocations, 1, "same padded size across formats");

        let taller = source(&gpu.device, wgpu::TextureFormat::Rgba8Unorm, 2, 2);
        let taller_bytes = [
            10u8, 11, 12, 13, 14, 15, 16, 17, 20, 21, 22, 23, 24, 25, 26, 27,
        ];
        upload(&gpu.queue, &taller, &taller_bytes);
        let got = readback
            .with_mapped_rows(
                &gpu.device,
                &gpu.queue,
                &taller,
                |data, row, stride, height| {
                    (0..height)
                        .flat_map(|y| data[y * stride..y * stride + row].iter().copied())
                        .collect::<Vec<_>>()
                },
            )
            .unwrap();
        assert_eq!(got.as_slice(), taller_bytes.as_slice());
        assert_eq!(readback.allocations, 2, "taller copy grows once");
        readback.trim(512);
        assert_eq!(readback.capacity_bytes(), 512);
        readback.trim(256);
        assert_eq!(readback.capacity_bytes(), 0);
        assert!(readback.buffer.is_none());
        let got = readback
            .with_mapped_rows(&gpu.device, &gpu.queue, &half, |data, row, _, _| {
                data[..row].to_vec()
            })
            .unwrap();
        assert_eq!(got.as_slice(), half_bytes.as_slice());
        assert_eq!(readback.allocations, 3, "trimmed staging is recreated");
    }

    #[test]
    fn ring_copies_two_fresh_frames_and_reuses_unmapped_slots() {
        let gpu = crate::Gpu::new("readback ring test").unwrap();
        let mut ring = ReadbackRing::default();
        let texture = source(&gpu.device, wgpu::TextureFormat::Rgba8Unorm, 2, 2);
        let first = [1u8; 16];
        let second = [2u8; 16];
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        upload(&gpu.queue, &texture, &first);
        let (a, a_copy) = ring.reserve(&gpu.device, &texture, 1024).unwrap().unwrap();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        a_copy.record(&mut encoder, &texture);
        gpu.queue.submit([encoder.finish()]);
        let first_ready = ready_tx.clone();
        ring.start_map(a, move || {
            let _ = first_ready.send(0);
        });
        upload(&gpu.queue, &texture, &second);
        let (b, b_copy) = ring.reserve(&gpu.device, &texture, 1024).unwrap().unwrap();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        b_copy.record(&mut encoder, &texture);
        gpu.queue.submit([encoder.finish()]);
        ring.start_map(b, move || {
            let _ = ready_tx.send(1);
        });
        assert!(ring.reserve(&gpu.device, &texture, 1024).unwrap().is_none());
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        let mut notified = [ready_rx.try_recv().unwrap(), ready_rx.try_recv().unwrap()];
        notified.sort();
        assert_eq!(notified, [0, 1]);
        let unpack = |data: &[u8], row: usize, stride: usize, height: usize| {
            (0..height)
                .flat_map(|y| data[y * stride..y * stride + row].iter().copied())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ring.try_with_mapped_rows(a, unpack, 1024).unwrap().unwrap(),
            first
        );
        assert_eq!(
            ring.try_with_mapped_rows(b, unpack, 1024).unwrap().unwrap(),
            second
        );
        assert!(!ring.in_flight());
        assert_eq!(ring.allocations, 2);
        let (c, _) = ring.reserve(&gpu.device, &texture, 1024).unwrap().unwrap();
        assert_eq!(ring.allocations, 2, "unmapped slot reused");
        ring.cancel(c, 1024);
        assert!(!ring.in_flight());

        let half = source(&gpu.device, wgpu::TextureFormat::Rgba16Float, 2, 2);
        let half_bytes = [0x00u8, 0x3c, 0, 0, 0, 0, 0, 0x3c].repeat(4);
        upload(&gpu.queue, &half, &half_bytes);
        let (mixed, copy) = ring.reserve(&gpu.device, &half, 1024).unwrap().unwrap();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        copy.record(&mut encoder, &half);
        gpu.queue.submit([encoder.finish()]);
        ring.start_map(mixed, || {});
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let raw = ring
            .try_with_mapped_rows(mixed, unpack, 1024)
            .unwrap()
            .unwrap();
        assert_eq!(raw, half_bytes, "format change uses a fresh copy");
        assert_eq!(ring.allocations, 2, "same padded size reuses the slot");

        // A session can close after submission but before the map callback.
        // Reusing its cancelled slot must not read the cancelled frame.
        upload(&gpu.queue, &texture, &first);
        let (cancelled, copy) = ring.reserve(&gpu.device, &texture, 1024).unwrap().unwrap();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        copy.record(&mut encoder, &texture);
        gpu.queue.submit([encoder.finish()]);
        ring.start_map(cancelled, || {});
        ring.cancel(cancelled, 1024);
        upload(&gpu.queue, &texture, &second);
        let (next, copy) = ring.reserve(&gpu.device, &texture, 1024).unwrap().unwrap();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        copy.record(&mut encoder, &texture);
        gpu.queue.submit([encoder.finish()]);
        ring.start_map(next, || {});
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        assert_eq!(
            ring.try_with_mapped_rows(next, unpack, 1024)
                .unwrap()
                .unwrap(),
            second,
        );
    }

    #[test]
    fn ring_serializes_oversized_copy_and_trims_its_buffer() {
        let gpu = crate::Gpu::new("readback ring budget test").unwrap();
        let mut ring = ReadbackRing::default();
        let texture = source(&gpu.device, wgpu::TextureFormat::Rgba16Float, 2, 2);
        let (ticket, _) = ring.reserve(&gpu.device, &texture, 512).unwrap().unwrap();
        let big = source(&gpu.device, wgpu::TextureFormat::Rgba16Float, 2, 3);
        assert!(!ring.can_reserve(768, 512));
        ring.cancel(ticket, 512);
        let bytes = [7u8; 48];
        upload(&gpu.queue, &big, &bytes);
        let (large, copy) = ring.reserve(&gpu.device, &big, 512).unwrap().unwrap();
        assert_eq!(ring.capacity_bytes(), 768);
        assert!(!ring.can_reserve(512, 512));
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        copy.record(&mut encoder, &big);
        gpu.queue.submit([encoder.finish()]);
        ring.start_map(large, || {});
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let got = ring
            .try_with_mapped_rows(
                large,
                |data, row, stride, height| {
                    (0..height)
                        .flat_map(|y| data[y * stride..y * stride + row].iter().copied())
                        .collect::<Vec<_>>()
                },
                512,
            )
            .unwrap()
            .unwrap();
        assert_eq!(got, bytes);
        assert_eq!(ring.capacity_bytes(), 0, "large mapped allocation released");
    }

    #[test]
    fn half_round_trips() {
        for h in 0..=u16::MAX {
            let f = half_to_f32(h);
            if f.is_nan() {
                continue;
            }
            assert_eq!(f32_to_half(f), h, "{h:#06x} {f}");
        }
        assert_eq!(f32_to_half(1.0), 0x3c00);
        assert_eq!(f32_to_half(65520.0), 0x7c00);
        // 1 + 2^-11 is halfway between 1 and the next half: ties to even (1).
        assert_eq!(f32_to_half(1.0 + 1.0 / 2048.0), 0x3c00);
        assert_eq!(f32_to_half(1.0 + 3.0 / 2048.0), 0x3c02);
    }
}
