//! Wire format of the frame bridge. Every message the bridge receives is parsed by
//! [`parse`] from the raw receive buffer (header, optional descriptors, payload,
//! audit trailer). The parser is pure and total: it never panics on any input, which
//! the fuzz tests check, because it runs inside Electron main.

use crate::ffi::{
    MACH_MSG_PORT_DESCRIPTOR, MACH_MSG_TYPE_PORT_SEND, MACH_MSGH_BITS_COMPLEX, MACH_PORT_DEAD, MACH_PORT_NULL,
    audit_token_t, mach_port_t,
};

pub const MAGIC: u32 = 0x5331_4252; // "S1BR"
pub const VERSION: u32 = 1;

pub const ID_CHECKIN: i32 = 0x5331_0001;
pub const ID_CHECKIN_REPLY: i32 = 0x5331_0002;
pub const ID_SURFACE: i32 = 0x5331_0003;
pub const ID_FRAME: i32 = 0x5331_0004;
pub const ID_GOODBYE: i32 = 0x5331_0005;

pub const HEADER_LEN: usize = 24;
pub const BODY_LEN: usize = 4;
pub const PORT_DESC_LEN: usize = 12;
pub const AUDIT_TRAILER_LEN: usize = 52;

pub const CHECKIN_PAYLOAD: usize = 12; // magic, version, claimed pid
pub const REPLY_PAYLOAD: usize = 8; // magic, status
pub const SURFACE_PAYLOAD: usize = 4 + 6 * 4; // magic + SurfaceInfo
pub const FRAME_PAYLOAD: usize = 4 + FrameDescriptor::LEN;
pub const GOODBYE_PAYLOAD: usize = 4;

pub const fn complex_len(payload: usize) -> usize {
    HEADER_LEN + BODY_LEN + PORT_DESC_LEN + payload
}
pub const fn simple_len(payload: usize) -> usize {
    HEADER_LEN + payload
}

/// Largest message the bridge accepts, plus the audit trailer.
pub const MAX_RECEIVE: usize = {
    let a = complex_len(SURFACE_PAYLOAD);
    let b = simple_len(FRAME_PAYLOAD);
    (if a > b { a } else { b }) + AUDIT_TRAILER_LEN + 64
};

/// Pixel formats the pool may use. Values are part of the wire format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum PixelFormat {
    /// 8-bit BGRA, sRGB-encoded (E), blue-noise dithered in the display transform.
    Bgra8 = 0,
    /// Half-float RGBA, extended sRGB-encoded (E extended to all reals).
    Rgba16F = 1,
}

impl PixelFormat {
    pub fn from_u32(v: u32) -> Option<Self> {
        match v {
            0 => Some(Self::Bgra8),
            1 => Some(Self::Rgba16F),
            _ => None,
        }
    }
    pub fn bytes_per_pixel(self) -> u32 {
        match self {
            Self::Bgra8 => 4,
            Self::Rgba16F => 8,
        }
    }
    pub fn electron_name(self) -> &'static str {
        match self {
            Self::Bgra8 => "bgra",
            Self::Rgba16F => "rgbaf16",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceInfo {
    pub pool_index: u32,
    pub generation: u32,
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub bytes_per_row: u32,
}

pub const MAX_POOL: u32 = 64;
pub const MAX_DIMENSION: u32 = 16384;

/// Per-frame descriptor, sent after the command buffer that wrote the surface has
/// completed. Times are microseconds on the host's monotonic clock
/// (`CLOCK_UPTIME_RAW`, the clock Chromium's `TimeTicks` uses on macOS).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameDescriptor {
    pub pool_index: u32,
    pub generation: u32,
    pub seq: u64,
    pub pts_us: i64,
    pub duration_us: i64,
    pub revision: u64,
    /// Timestamp of the newest input event folded into this frame's revision.
    pub input_ts_us: i64,
    pub render_start_us: i64,
    pub gpu_done_us: i64,
    pub color_space: u32,
    pub flags: u32,
    /// Overlay geometry at this revision: the dragged marker's rect in frame pixels.
    pub overlay: [f32; 4],
}

pub const FLAG_GESTURE: u32 = 1;
pub const FLAG_SCENE_3D: u32 = 2;

impl FrameDescriptor {
    pub const LEN: usize = 4 * 2 + 8 * 7 + 4 * 2 + 4 * 4;

    pub fn encode(&self) -> [u8; Self::LEN] {
        let mut out = [0u8; Self::LEN];
        let mut w = Writer { buf: &mut out, at: 0 };
        w.u32(self.pool_index);
        w.u32(self.generation);
        w.u64(self.seq);
        w.i64(self.pts_us);
        w.i64(self.duration_us);
        w.u64(self.revision);
        w.i64(self.input_ts_us);
        w.i64(self.render_start_us);
        w.i64(self.gpu_done_us);
        w.u32(self.color_space);
        w.u32(self.flags);
        for v in self.overlay {
            w.u32(v.to_bits());
        }
        debug_assert_eq!(w.at, Self::LEN);
        out
    }

    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() != Self::LEN {
            return None;
        }
        let mut r = Reader { buf: b, at: 0 };
        let d = Self {
            pool_index: r.u32()?,
            generation: r.u32()?,
            seq: r.u64()?,
            pts_us: r.i64()?,
            duration_us: r.i64()?,
            revision: r.u64()?,
            input_ts_us: r.i64()?,
            render_start_us: r.i64()?,
            gpu_done_us: r.i64()?,
            color_space: r.u32()?,
            flags: r.u32()?,
            overlay: [
                f32::from_bits(r.u32()?),
                f32::from_bits(r.u32()?),
                f32::from_bits(r.u32()?),
                f32::from_bits(r.u32()?),
            ],
        };
        if d.pool_index >= MAX_POOL || d.overlay.iter().any(|v| !v.is_finite()) {
            return None;
        }
        Some(d)
    }
}

#[derive(Debug, PartialEq)]
pub enum Incoming {
    /// Check-in on the bootstrap service port; carries a send right to the engine's port.
    CheckIn { engine_port: mach_port_t, claimed_pid: u32 },
    /// Reply to a check-in (received by the engine); carries a send right to main's anonymous port.
    CheckInReply { bridge_port: mach_port_t, status: u32 },
    /// One pool surface; carries a send right made by `IOSurfaceCreateMachPort`.
    Surface { surface_port: mach_port_t, info: SurfaceInfo },
    Frame(FrameDescriptor),
    Goodbye,
}

impl Incoming {
    /// The right carried by this message, if any; callers release it on rejection.
    pub fn carried_port(&self) -> Option<mach_port_t> {
        match *self {
            Incoming::CheckIn { engine_port, .. } => Some(engine_port),
            Incoming::CheckInReply { bridge_port, .. } => Some(bridge_port),
            Incoming::Surface { surface_port, .. } => Some(surface_port),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("buffer shorter than a header")]
    Short,
    #[error("msgh_size {0} is inconsistent with the buffer")]
    Size(u32),
    #[error("unknown message id {0:#x}")]
    UnknownId(i32),
    #[error("message length {got} does not match {want} for its id")]
    Length { got: usize, want: usize },
    #[error("complex bit does not match the message id")]
    Complexity,
    #[error("unexpected reply port in the header")]
    ReplyPort,
    #[error("descriptor count {0} is not 1")]
    DescriptorCount(u32),
    #[error("descriptor is not a port send right")]
    Descriptor,
    #[error("bad magic or protocol version")]
    Magic,
    #[error("invalid field: {0}")]
    Field(&'static str),
    #[error("missing or malformed audit trailer")]
    Trailer,
}

/// Parses a received message. `buf` is the receive buffer; the kernel places the
/// audit trailer at `round4(msgh_size)`.
pub fn parse(buf: &[u8]) -> Result<(Incoming, audit_token_t), ParseError> {
    if buf.len() < HEADER_LEN {
        return Err(ParseError::Short);
    }
    let bits = rd_u32(buf, 0);
    let size = rd_u32(buf, 4);
    let remote = rd_u32(buf, 8);
    let id = rd_u32(buf, 20) as i32;
    let size_us = size as usize;
    if size_us < HEADER_LEN || !size_us.is_multiple_of(4) || size_us > buf.len() {
        return Err(ParseError::Size(size));
    }
    let trailer_at = size_us; // already 4-aligned
    let audit = parse_trailer(buf.get(trailer_at..).ok_or(ParseError::Trailer)?)?;

    // The bridge never uses reply ports; a message carrying one is rejected so the
    // right is destroyed by the caller instead of leaking.
    if remote != MACH_PORT_NULL || (bits & 0x1f) != 0 {
        return Err(ParseError::ReplyPort);
    }
    let complex = bits & MACH_MSGH_BITS_COMPLEX != 0;
    let msg = &buf[..size_us];

    let (want_complex, payload_len) = match id {
        ID_CHECKIN => (true, CHECKIN_PAYLOAD),
        ID_CHECKIN_REPLY => (true, REPLY_PAYLOAD),
        ID_SURFACE => (true, SURFACE_PAYLOAD),
        ID_FRAME => (false, FRAME_PAYLOAD),
        ID_GOODBYE => (false, GOODBYE_PAYLOAD),
        other => return Err(ParseError::UnknownId(other)),
    };
    if complex != want_complex {
        return Err(ParseError::Complexity);
    }
    let want = if want_complex { complex_len(payload_len) } else { simple_len(payload_len) };
    if msg.len() != want {
        return Err(ParseError::Length { got: msg.len(), want });
    }

    let (port, payload) = if want_complex {
        let count = rd_u32(msg, HEADER_LEN);
        if count != 1 {
            return Err(ParseError::DescriptorCount(count));
        }
        let d = HEADER_LEN + BODY_LEN;
        let name = rd_u32(msg, d);
        let disposition = msg[d + 10];
        let type_ = msg[d + 11];
        if type_ != MACH_MSG_PORT_DESCRIPTOR
            || u32::from(disposition) != MACH_MSG_TYPE_PORT_SEND
            || name == MACH_PORT_NULL
            || name == MACH_PORT_DEAD
        {
            return Err(ParseError::Descriptor);
        }
        (Some(name), &msg[d + PORT_DESC_LEN..])
    } else {
        (None, &msg[HEADER_LEN..])
    };

    if rd_u32(payload, 0) != MAGIC {
        return Err(ParseError::Magic);
    }
    let p = &payload[4..];
    let incoming = match id {
        ID_CHECKIN => {
            if rd_u32(p, 0) != VERSION {
                return Err(ParseError::Magic);
            }
            Incoming::CheckIn { engine_port: port.unwrap_or_default(), claimed_pid: rd_u32(p, 4) }
        }
        ID_CHECKIN_REPLY => Incoming::CheckInReply { bridge_port: port.unwrap_or_default(), status: rd_u32(p, 0) },
        ID_SURFACE => {
            let info = SurfaceInfo {
                pool_index: rd_u32(p, 0),
                generation: rd_u32(p, 4),
                width: rd_u32(p, 8),
                height: rd_u32(p, 12),
                format: PixelFormat::from_u32(rd_u32(p, 16)).ok_or(ParseError::Field("format"))?,
                bytes_per_row: rd_u32(p, 20),
            };
            validate_surface(&info)?;
            Incoming::Surface { surface_port: port.unwrap_or_default(), info }
        }
        ID_FRAME => Incoming::Frame(FrameDescriptor::decode(p).ok_or(ParseError::Field("frame descriptor"))?),
        ID_GOODBYE => Incoming::Goodbye,
        _ => unreachable!("id matched above"),
    };
    Ok((incoming, audit))
}

pub fn validate_surface(info: &SurfaceInfo) -> Result<(), ParseError> {
    if info.pool_index >= MAX_POOL {
        return Err(ParseError::Field("pool_index"));
    }
    if info.width == 0 || info.height == 0 || info.width > MAX_DIMENSION || info.height > MAX_DIMENSION {
        return Err(ParseError::Field("dimensions"));
    }
    let min_row = u64::from(info.width) * u64::from(info.format.bytes_per_pixel());
    if u64::from(info.bytes_per_row) < min_row || info.bytes_per_row > 4 * 65536 {
        return Err(ParseError::Field("bytes_per_row"));
    }
    Ok(())
}

fn parse_trailer(t: &[u8]) -> Result<audit_token_t, ParseError> {
    if t.len() < AUDIT_TRAILER_LEN {
        return Err(ParseError::Trailer);
    }
    let ty = rd_u32(t, 0);
    let size = rd_u32(t, 4) as usize;
    if ty != 0 || size < AUDIT_TRAILER_LEN || size > t.len() {
        return Err(ParseError::Trailer);
    }
    // type, size, seqno, sender (2 words), then the audit token (8 words).
    let mut val = [0u32; 8];
    for (i, v) in val.iter_mut().enumerate() {
        *v = rd_u32(t, 20 + 4 * i);
    }
    Ok(audit_token_t { val })
}

// --- Encoding (senders) -------------------------------------------------------------

/// A message ready for `mach_msg(MACH_SEND_MSG)`. 4-byte aligned storage.
pub struct OutMessage {
    words: Vec<u32>,
}

impl OutMessage {
    pub fn as_mut_ptr(&mut self) -> *mut crate::ffi::mach_msg_header_t {
        self.words.as_mut_ptr().cast()
    }
    pub fn len(&self) -> usize {
        self.words.len() * 4
    }
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }
    pub fn bytes(&self) -> Vec<u8> {
        self.words.iter().flat_map(|w| w.to_ne_bytes()).collect()
    }
}

fn header(bits: u32, size: usize, remote: mach_port_t, id: i32) -> [u32; 6] {
    [bits, size as u32, remote, 0, 0, id as u32]
}

fn port_desc(name: mach_port_t, disposition: u32) -> [u32; 3] {
    // pad2 (16 bits), disposition (8), type (8), little-endian within the last word.
    [name, 0, (disposition << 16) | (u32::from(MACH_MSG_PORT_DESCRIPTOR) << 24)]
}

fn complex(remote: mach_port_t, id: i32, port: mach_port_t, disposition: u32, payload: &[u32]) -> OutMessage {
    let size = complex_len(payload.len() * 4);
    let bits = crate::ffi::mach_msgh_bits(crate::ffi::MACH_MSG_TYPE_COPY_SEND, 0) | MACH_MSGH_BITS_COMPLEX;
    let mut words = Vec::with_capacity(size / 4);
    words.extend_from_slice(&header(bits, size, remote, id));
    words.push(1);
    words.extend_from_slice(&port_desc(port, disposition));
    words.extend_from_slice(payload);
    OutMessage { words }
}

fn simple(remote: mach_port_t, id: i32, payload: &[u32]) -> OutMessage {
    let size = simple_len(payload.len() * 4);
    let bits = crate::ffi::mach_msgh_bits(crate::ffi::MACH_MSG_TYPE_COPY_SEND, 0);
    let mut words = Vec::with_capacity(size / 4);
    words.extend_from_slice(&header(bits, size, remote, id));
    words.extend_from_slice(payload);
    OutMessage { words }
}

pub fn encode_checkin(service: mach_port_t, engine_receive: mach_port_t, pid: u32) -> OutMessage {
    complex(service, ID_CHECKIN, engine_receive, crate::ffi::MACH_MSG_TYPE_MAKE_SEND, &[MAGIC, VERSION, pid])
}

pub fn encode_checkin_reply(engine: mach_port_t, bridge_receive: mach_port_t, status: u32) -> OutMessage {
    complex(engine, ID_CHECKIN_REPLY, bridge_receive, crate::ffi::MACH_MSG_TYPE_MAKE_SEND, &[MAGIC, status])
}

/// `surface_port` is moved: the sender gives up its send right.
pub fn encode_surface(bridge: mach_port_t, surface_port: mach_port_t, info: &SurfaceInfo) -> OutMessage {
    complex(
        bridge,
        ID_SURFACE,
        surface_port,
        crate::ffi::MACH_MSG_TYPE_MOVE_SEND,
        &[
            MAGIC,
            info.pool_index,
            info.generation,
            info.width,
            info.height,
            info.format as u32,
            info.bytes_per_row,
        ],
    )
}

pub fn encode_frame(bridge: mach_port_t, desc: &FrameDescriptor) -> OutMessage {
    let bytes = desc.encode();
    let mut payload = vec![MAGIC];
    payload.extend(bytes.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])));
    simple(bridge, ID_FRAME, &payload)
}

pub fn encode_goodbye(bridge: mach_port_t) -> OutMessage {
    simple(bridge, ID_GOODBYE, &[MAGIC])
}

// --- helpers ----------------------------------------------------------------------

/// Reads a native-endian u32; out-of-range reads yield 0 (callers check lengths first,
/// this keeps the parser total).
fn rd_u32(b: &[u8], at: usize) -> u32 {
    match b.get(at..at + 4) {
        Some(s) => u32::from_ne_bytes([s[0], s[1], s[2], s[3]]),
        None => 0,
    }
}

struct Writer<'a> {
    buf: &'a mut [u8],
    at: usize,
}
impl Writer<'_> {
    fn put(&mut self, b: &[u8]) {
        self.buf[self.at..self.at + b.len()].copy_from_slice(b);
        self.at += b.len();
    }
    fn u32(&mut self, v: u32) {
        self.put(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.put(&v.to_le_bytes());
    }
    fn i64(&mut self, v: i64) {
        self.put(&v.to_le_bytes());
    }
}

struct Reader<'a> {
    buf: &'a [u8],
    at: usize,
}
impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let s = self.buf.get(self.at..self.at + N)?;
        self.at += N;
        s.try_into().ok()
    }
    fn u32(&mut self) -> Option<u32> {
        self.take::<4>().map(u32::from_le_bytes)
    }
    fn u64(&mut self) -> Option<u64> {
        self.take::<8>().map(u64::from_le_bytes)
    }
    fn i64(&mut self) -> Option<i64> {
        self.take::<8>().map(i64::from_le_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simulates what the kernel delivers: the sent bytes with the descriptor's
    /// disposition rewritten to its received form, and an audit trailer appended.
    pub(crate) fn as_received(msg: &OutMessage, token: [u32; 8]) -> Vec<u8> {
        let mut b = msg.bytes();
        // Clear the remote-port bits and port (the kernel swaps remote/local).
        let bits = rd_u32(&b, 0) & MACH_MSGH_BITS_COMPLEX;
        b[0..4].copy_from_slice(&bits.to_ne_bytes());
        b[8..12].copy_from_slice(&0u32.to_ne_bytes());
        if bits != 0 {
            let d = HEADER_LEN + BODY_LEN;
            b[d + 10] = MACH_MSG_TYPE_PORT_SEND as u8;
        }
        let mut t = vec![0u8; AUDIT_TRAILER_LEN];
        t[4..8].copy_from_slice(&(AUDIT_TRAILER_LEN as u32).to_ne_bytes());
        for (i, v) in token.iter().enumerate() {
            t[20 + 4 * i..24 + 4 * i].copy_from_slice(&v.to_ne_bytes());
        }
        b.extend_from_slice(&t);
        b
    }

    fn sample_desc() -> FrameDescriptor {
        FrameDescriptor {
            pool_index: 3,
            generation: 2,
            seq: 1234,
            pts_us: 99_000,
            duration_us: 16_667,
            revision: 77,
            input_ts_us: 98_000,
            render_start_us: 97_000,
            gpu_done_us: 97_500,
            color_space: 1,
            flags: FLAG_GESTURE,
            overlay: [10.0, 20.0, 30.0, 40.0],
        }
    }

    #[test]
    fn frame_round_trip() {
        let d = sample_desc();
        let rx = as_received(&encode_frame(5, &d), [1, 2, 3, 4, 5, 6, 7, 8]);
        let (msg, token) = parse(&rx).unwrap();
        assert_eq!(msg, Incoming::Frame(d));
        assert_eq!(token.val, [1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn surface_round_trip_and_validation() {
        let info = SurfaceInfo {
            pool_index: 7,
            generation: 1,
            width: 3840,
            height: 2160,
            format: PixelFormat::Rgba16F,
            bytes_per_row: 3840 * 8,
        };
        let rx = as_received(&encode_surface(5, 0x1003, &info), [0; 8]);
        assert_eq!(parse(&rx).unwrap().0, Incoming::Surface { surface_port: 0x1003, info });

        let bad = SurfaceInfo { bytes_per_row: 3840 * 4, ..info };
        let rx = as_received(&encode_surface(5, 0x1003, &bad), [0; 8]);
        assert_eq!(parse(&rx).unwrap_err(), ParseError::Field("bytes_per_row"));
        let bad = SurfaceInfo { pool_index: MAX_POOL, ..info };
        let rx = as_received(&encode_surface(5, 0x1003, &bad), [0; 8]);
        assert_eq!(parse(&rx).unwrap_err(), ParseError::Field("pool_index"));
    }

    #[test]
    fn checkin_round_trip_and_rejections() {
        let rx = as_received(&encode_checkin(5, 0x2003, 4242), [0; 8]);
        assert_eq!(parse(&rx).unwrap().0, Incoming::CheckIn { engine_port: 0x2003, claimed_pid: 4242 });

        // Wrong magic.
        let mut m = rx.clone();
        let at = complex_len(0);
        m[at] ^= 1;
        assert_eq!(parse(&m).unwrap_err(), ParseError::Magic);
        // A reply port in the header.
        let mut m = rx.clone();
        m[8..12].copy_from_slice(&0x77u32.to_ne_bytes());
        assert_eq!(parse(&m).unwrap_err(), ParseError::ReplyPort);
        // Complex bit stripped.
        let mut m = rx.clone();
        let bits = rd_u32(&m, 0) & !MACH_MSGH_BITS_COMPLEX;
        m[0..4].copy_from_slice(&bits.to_ne_bytes());
        assert_eq!(parse(&m).unwrap_err(), ParseError::Complexity);
        // A receive right instead of a send right.
        let mut m = rx.clone();
        m[HEADER_LEN + BODY_LEN + 10] = crate::ffi::MACH_MSG_TYPE_MOVE_RECEIVE as u8;
        assert_eq!(parse(&m).unwrap_err(), ParseError::Descriptor);
        // Truncated trailer.
        assert_eq!(parse(&rx[..rx.len() - 1]).unwrap_err(), ParseError::Trailer);
    }

    #[test]
    fn non_finite_overlay_rejected() {
        let mut d = sample_desc();
        d.overlay[2] = f32::NAN;
        let rx = as_received(&encode_frame(5, &d), [0; 8]);
        assert_eq!(parse(&rx).unwrap_err(), ParseError::Field("frame descriptor"));
    }

    /// Deterministic fuzzing: random buffers and single/multi-byte mutations of every
    /// valid message kind. The parser must never panic, and anything it accepts must
    /// re-encode to an equivalent message.
    #[test]
    fn fuzz_parser_total() {
        let mut rng = XorShift(0x9e37_79b9_7f4a_7c15);
        let info = SurfaceInfo {
            pool_index: 1,
            generation: 0,
            width: 64,
            height: 32,
            format: PixelFormat::Bgra8,
            bytes_per_row: 256,
        };
        let seeds = [
            as_received(&encode_checkin(5, 0x2003, 1), [9; 8]),
            as_received(&encode_checkin_reply(5, 0x2003, 0), [9; 8]),
            as_received(&encode_surface(5, 0x1003, &info), [9; 8]),
            as_received(&encode_frame(5, &sample_desc()), [9; 8]),
            as_received(&encode_goodbye(5), [9; 8]),
        ];
        let mut accepted = 0usize;
        for i in 0..400_000u32 {
            let mut buf = if i % 8 == 0 {
                let n = (rng.next() % (MAX_RECEIVE as u64 + 32)) as usize;
                (0..n).map(|_| rng.next() as u8).collect::<Vec<_>>()
            } else {
                seeds[(rng.next() % seeds.len() as u64) as usize].clone()
            };
            let flips = 1 + rng.next() % 4;
            for _ in 0..flips {
                if buf.is_empty() {
                    break;
                }
                let at = (rng.next() % buf.len() as u64) as usize;
                match rng.next() % 3 {
                    0 => buf[at] = rng.next() as u8,
                    1 => buf[at] ^= 1 << (rng.next() % 8),
                    _ => buf.truncate(at),
                }
            }
            if let Ok((msg, _)) = parse(&buf) {
                accepted += 1;
                match msg {
                    Incoming::Frame(d) => assert_eq!(FrameDescriptor::decode(&d.encode()), Some(d)),
                    Incoming::Surface { info, .. } => assert!(validate_surface(&info).is_ok()),
                    _ => {}
                }
            }
        }
        // Mutations that only touch payload values are legitimately accepted.
        assert!(accepted > 0);
    }

    struct XorShift(u64);
    impl XorShift {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
    }
}
