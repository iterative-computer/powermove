//! Wire format of the frame bridge's Mach messages (plan section 18).
//!
//! Only control traffic crosses Mach: the engine's one check-in, main's reply,
//! each pool surface (once, carrying the `IOSurfaceCreateMachPort` send
//! right), the retirement of a pool generation, and a goodbye. Per-frame
//! descriptors never pass through main (amendment 4): they travel from the
//! engine to the viewer over the viewer's `MessageChannelMain` port.
//!
//! Every message main receives is parsed by [`parse`] from the raw receive
//! buffer (header, optional port descriptor, payload, audit trailer). The
//! parser runs inside Electron main, so it is pure and total: it never panics
//! on any input, which the fuzz test checks.

use crate::ffi::{
    MACH_MSG_PORT_DESCRIPTOR, MACH_MSG_TYPE_PORT_SEND, MACH_MSGH_BITS_COMPLEX, MACH_PORT_DEAD,
    MACH_PORT_NULL, audit_token_t, mach_port_t,
};

/// "PMBR".
pub const MAGIC: u32 = 0x504D_4252;
/// Bumped on any change to a message layout.
pub const VERSION: u32 = 1;

pub const ID_CHECKIN: i32 = 0x504D_0001;
pub const ID_CHECKIN_REPLY: i32 = 0x504D_0002;
pub const ID_SURFACE: i32 = 0x504D_0003;
pub const ID_RETIRE: i32 = 0x504D_0004;
pub const ID_GOODBYE: i32 = 0x504D_0005;

pub const HEADER_LEN: usize = 24;
pub const BODY_LEN: usize = 4;
pub const PORT_DESC_LEN: usize = 12;
/// `mach_msg_audit_trailer_t`: type, size, seqno, sender (2 words), audit token (8 words).
pub const AUDIT_TRAILER_LEN: usize = 52;

/// magic, version, claimed pid.
pub const CHECKIN_PAYLOAD: usize = 12;
/// magic, status.
pub const REPLY_PAYLOAD: usize = 8;
/// magic + [`SurfaceInfo`] (8 words).
pub const SURFACE_PAYLOAD: usize = 4 + SurfaceInfo::WORDS * 4;
/// magic, client port, generation.
pub const RETIRE_PAYLOAD: usize = 12;
/// magic.
pub const GOODBYE_PAYLOAD: usize = 4;

pub const fn complex_len(payload: usize) -> usize {
    HEADER_LEN + BODY_LEN + PORT_DESC_LEN + payload
}

pub const fn simple_len(payload: usize) -> usize {
    HEADER_LEN + payload
}

/// Largest message the bridge accepts, plus the audit trailer and slack.
pub const MAX_RECEIVE: usize = {
    let a = complex_len(SURFACE_PAYLOAD);
    let b = simple_len(RETIRE_PAYLOAD);
    (if a > b { a } else { b }) + AUDIT_TRAILER_LEN + 64
};

/// Pool pixel formats. Values are part of the wire format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum PixelFormat {
    /// 8-bit BGRA holding display-encoded values, blue-noise dithered by the
    /// display transform.
    Bgra8 = 0,
    /// Half-float RGBA holding display-encoded values extended past 1.0 (EDR).
    Rgba16F = 1,
}

impl PixelFormat {
    pub fn from_u32(value: u32) -> Option<Self> {
        match value {
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

    /// Electron's `sharedTexture` pixel format name.
    pub fn electron_name(self) -> &'static str {
        match self {
            Self::Bgra8 => "bgra",
            Self::Rgba16F => "rgbaf16",
        }
    }
}

/// One pool surface as main learns it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceInfo {
    /// The engine's id for the client port whose viewer the pool serves
    /// (main assigned it, so main knows the window).
    pub client_port: u32,
    /// The pool generation (unique per engine launch, increasing).
    pub generation: u32,
    pub pool_index: u32,
    /// Surfaces in this generation.
    pub pool_size: u32,
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub bytes_per_row: u32,
}

impl SurfaceInfo {
    pub const WORDS: usize = 8;

    fn words(&self) -> [u32; Self::WORDS] {
        [
            self.client_port,
            self.generation,
            self.pool_index,
            self.pool_size,
            self.width,
            self.height,
            self.format as u32,
            self.bytes_per_row,
        ]
    }
}

/// Most surfaces in one pool (contract 3 allows 4 to 32).
pub const MAX_POOL: u32 = 32;
pub const MAX_DIMENSION: u32 = 16384;

#[derive(Debug, PartialEq, Eq)]
pub enum Incoming {
    /// On the bootstrap service port; carries a send right to the engine's port.
    CheckIn {
        engine_port: mach_port_t,
        claimed_pid: u32,
    },
    /// Received by the engine; carries a send right to main's anonymous port.
    CheckInReply {
        bridge_port: mach_port_t,
        status: u32,
    },
    /// One pool surface; carries the send right made by `IOSurfaceCreateMachPort`.
    Surface {
        surface_port: mach_port_t,
        info: SurfaceInfo,
    },
    /// The engine stopped using a pool generation (resize, format change, or
    /// the viewer's port closed); main may release its imports.
    Retire {
        client_port: u32,
        generation: u32,
    },
    Goodbye,
}

impl Incoming {
    /// The right a message carries, if any; callers release it on rejection.
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

/// Parses a received message. `buf` is the receive buffer; the kernel puts
/// the audit trailer at `msgh_size` (always 4-aligned).
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
    let audit = parse_trailer(buf.get(size_us..).ok_or(ParseError::Trailer)?)?;

    // The bridge never uses reply ports; a message carrying one is rejected so
    // the caller destroys the right instead of leaking it.
    if remote != MACH_PORT_NULL || (bits & 0x1f) != 0 {
        return Err(ParseError::ReplyPort);
    }
    let complex = bits & MACH_MSGH_BITS_COMPLEX != 0;
    let msg = &buf[..size_us];

    let (want_complex, payload_len) = match id {
        ID_CHECKIN => (true, CHECKIN_PAYLOAD),
        ID_CHECKIN_REPLY => (true, REPLY_PAYLOAD),
        ID_SURFACE => (true, SURFACE_PAYLOAD),
        ID_RETIRE => (false, RETIRE_PAYLOAD),
        ID_GOODBYE => (false, GOODBYE_PAYLOAD),
        other => return Err(ParseError::UnknownId(other)),
    };
    if complex != want_complex {
        return Err(ParseError::Complexity);
    }
    let want = if want_complex {
        complex_len(payload_len)
    } else {
        simple_len(payload_len)
    };
    if msg.len() != want {
        return Err(ParseError::Length {
            got: msg.len(),
            want,
        });
    }

    let (port, payload) = if want_complex {
        let count = rd_u32(msg, HEADER_LEN);
        if count != 1 {
            return Err(ParseError::DescriptorCount(count));
        }
        let d = HEADER_LEN + BODY_LEN;
        let name = rd_u32(msg, d);
        let disposition = msg[d + 10];
        let kind = msg[d + 11];
        if kind != MACH_MSG_PORT_DESCRIPTOR
            || u32::from(disposition) != MACH_MSG_TYPE_PORT_SEND
            || name == MACH_PORT_NULL
            || name == MACH_PORT_DEAD
        {
            return Err(ParseError::Descriptor);
        }
        (name, &msg[d + PORT_DESC_LEN..])
    } else {
        (MACH_PORT_NULL, &msg[HEADER_LEN..])
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
            Incoming::CheckIn {
                engine_port: port,
                claimed_pid: rd_u32(p, 4),
            }
        }
        ID_CHECKIN_REPLY => Incoming::CheckInReply {
            bridge_port: port,
            status: rd_u32(p, 0),
        },
        ID_SURFACE => {
            let info = SurfaceInfo {
                client_port: rd_u32(p, 0),
                generation: rd_u32(p, 4),
                pool_index: rd_u32(p, 8),
                pool_size: rd_u32(p, 12),
                width: rd_u32(p, 16),
                height: rd_u32(p, 20),
                format: PixelFormat::from_u32(rd_u32(p, 24)).ok_or(ParseError::Field("format"))?,
                bytes_per_row: rd_u32(p, 28),
            };
            validate_surface(&info)?;
            Incoming::Surface {
                surface_port: port,
                info,
            }
        }
        ID_RETIRE => Incoming::Retire {
            client_port: rd_u32(p, 0),
            generation: rd_u32(p, 4),
        },
        ID_GOODBYE => Incoming::Goodbye,
        _ => return Err(ParseError::UnknownId(id)),
    };
    Ok((incoming, audit))
}

/// Field checks shared by the sender and the parser.
pub fn validate_surface(info: &SurfaceInfo) -> Result<(), ParseError> {
    if info.pool_size == 0 || info.pool_size > MAX_POOL {
        return Err(ParseError::Field("pool_size"));
    }
    if info.pool_index >= info.pool_size {
        return Err(ParseError::Field("pool_index"));
    }
    if info.width == 0
        || info.height == 0
        || info.width > MAX_DIMENSION
        || info.height > MAX_DIMENSION
    {
        return Err(ParseError::Field("dimensions"));
    }
    let min_row = u64::from(info.width) * u64::from(info.format.bytes_per_pixel());
    // A row is at most the widest surface plus 64 KiB of alignment padding.
    let max_row = u64::from(MAX_DIMENSION) * 8 + 65536;
    if u64::from(info.bytes_per_row) < min_row || u64::from(info.bytes_per_row) > max_row {
        return Err(ParseError::Field("bytes_per_row"));
    }
    Ok(())
}

fn parse_trailer(t: &[u8]) -> Result<audit_token_t, ParseError> {
    if t.len() < AUDIT_TRAILER_LEN {
        return Err(ParseError::Trailer);
    }
    let kind = rd_u32(t, 0);
    let size = rd_u32(t, 4) as usize;
    if kind != 0 || size < AUDIT_TRAILER_LEN || size > t.len() {
        return Err(ParseError::Trailer);
    }
    let mut val = [0u32; 8];
    for (i, v) in val.iter_mut().enumerate() {
        *v = rd_u32(t, 20 + 4 * i);
    }
    Ok(audit_token_t { val })
}

// --- Encoding (senders) -----------------------------------------------------

/// A message ready for `mach_msg(MACH_SEND_MSG)`, in 4-byte aligned storage.
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
    // pad2 (16 bits), disposition (8), type (8), little-endian in the last word.
    [
        name,
        0,
        (disposition << 16) | (u32::from(MACH_MSG_PORT_DESCRIPTOR) << 24),
    ]
}

fn complex(
    remote: mach_port_t,
    id: i32,
    port: mach_port_t,
    disposition: u32,
    payload: &[u32],
) -> OutMessage {
    let size = complex_len(payload.len() * 4);
    let bits =
        crate::ffi::mach_msgh_bits(crate::ffi::MACH_MSG_TYPE_COPY_SEND, 0) | MACH_MSGH_BITS_COMPLEX;
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
    complex(
        service,
        ID_CHECKIN,
        engine_receive,
        crate::ffi::MACH_MSG_TYPE_MAKE_SEND,
        &[MAGIC, VERSION, pid],
    )
}

pub fn encode_checkin_reply(
    engine: mach_port_t,
    bridge_receive: mach_port_t,
    status: u32,
) -> OutMessage {
    complex(
        engine,
        ID_CHECKIN_REPLY,
        bridge_receive,
        crate::ffi::MACH_MSG_TYPE_MAKE_SEND,
        &[MAGIC, status],
    )
}

/// `surface_port` is moved: the sender gives up its send right.
pub fn encode_surface(
    bridge: mach_port_t,
    surface_port: mach_port_t,
    info: &SurfaceInfo,
) -> OutMessage {
    let mut payload = vec![MAGIC];
    payload.extend_from_slice(&info.words());
    complex(
        bridge,
        ID_SURFACE,
        surface_port,
        crate::ffi::MACH_MSG_TYPE_MOVE_SEND,
        &payload,
    )
}

pub fn encode_retire(bridge: mach_port_t, client_port: u32, generation: u32) -> OutMessage {
    simple(bridge, ID_RETIRE, &[MAGIC, client_port, generation])
}

pub fn encode_goodbye(bridge: mach_port_t) -> OutMessage {
    simple(bridge, ID_GOODBYE, &[MAGIC])
}

/// Reads a native-endian u32; out-of-range reads yield 0 (callers check
/// lengths first; this keeps the parser total).
fn rd_u32(b: &[u8], at: usize) -> u32 {
    match b.get(at..at.saturating_add(4)) {
        Some(s) => u32::from_ne_bytes([s[0], s[1], s[2], s[3]]),
        None => 0,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// What the kernel delivers: the sent bytes with the header's remote bits
    /// cleared, the descriptor's disposition in received form, and an audit
    /// trailer appended.
    pub(crate) fn as_received(msg: &OutMessage, token: [u32; 8]) -> Vec<u8> {
        let mut b = msg.bytes();
        let bits = rd_u32(&b, 0) & MACH_MSGH_BITS_COMPLEX;
        b[0..4].copy_from_slice(&bits.to_ne_bytes());
        b[8..12].copy_from_slice(&0u32.to_ne_bytes());
        if bits != 0 {
            b[HEADER_LEN + BODY_LEN + 10] = MACH_MSG_TYPE_PORT_SEND as u8;
        }
        let mut t = vec![0u8; AUDIT_TRAILER_LEN];
        t[4..8].copy_from_slice(&(AUDIT_TRAILER_LEN as u32).to_ne_bytes());
        for (i, v) in token.iter().enumerate() {
            t[20 + 4 * i..24 + 4 * i].copy_from_slice(&v.to_ne_bytes());
        }
        b.extend_from_slice(&t);
        b
    }

    pub(crate) fn sample_surface() -> SurfaceInfo {
        SurfaceInfo {
            client_port: 7,
            generation: 3,
            pool_index: 5,
            pool_size: 8,
            width: 3840,
            height: 2160,
            format: PixelFormat::Rgba16F,
            bytes_per_row: 3840 * 8,
        }
    }

    #[test]
    fn surface_round_trip_and_validation() {
        let info = sample_surface();
        let rx = as_received(&encode_surface(5, 0x1003, &info), [0; 8]);
        assert_eq!(
            parse(&rx).unwrap().0,
            Incoming::Surface {
                surface_port: 0x1003,
                info
            }
        );
        for (bad, field) in [
            (
                SurfaceInfo {
                    bytes_per_row: 3840 * 4,
                    ..info
                },
                "bytes_per_row",
            ),
            (
                SurfaceInfo {
                    pool_index: 8,
                    ..info
                },
                "pool_index",
            ),
            (
                SurfaceInfo {
                    pool_size: MAX_POOL + 1,
                    pool_index: 0,
                    ..info
                },
                "pool_size",
            ),
            (SurfaceInfo { width: 0, ..info }, "dimensions"),
            (
                SurfaceInfo {
                    height: MAX_DIMENSION + 1,
                    ..info
                },
                "dimensions",
            ),
        ] {
            let rx = as_received(&encode_surface(5, 0x1003, &bad), [0; 8]);
            assert_eq!(parse(&rx).unwrap_err(), ParseError::Field(field));
        }
    }

    #[test]
    fn retire_and_goodbye_round_trip() {
        let rx = as_received(&encode_retire(5, 9, 41), [1, 2, 3, 4, 5, 6, 7, 8]);
        let (msg, token) = parse(&rx).unwrap();
        assert_eq!(
            msg,
            Incoming::Retire {
                client_port: 9,
                generation: 41
            }
        );
        assert_eq!(token.val, [1, 2, 3, 4, 5, 6, 7, 8]);
        let rx = as_received(&encode_goodbye(5), [0; 8]);
        assert_eq!(parse(&rx).unwrap().0, Incoming::Goodbye);
    }

    #[test]
    fn checkin_round_trip_and_rejections() {
        let rx = as_received(&encode_checkin(5, 0x2003, 4242), [0; 8]);
        assert_eq!(
            parse(&rx).unwrap().0,
            Incoming::CheckIn {
                engine_port: 0x2003,
                claimed_pid: 4242
            }
        );
        // Wrong magic.
        let mut m = rx.clone();
        m[complex_len(0)] ^= 1;
        assert_eq!(parse(&m).unwrap_err(), ParseError::Magic);
        // Wrong version.
        let mut m = rx.clone();
        m[complex_len(4)] ^= 1;
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
        // A dead name.
        let mut m = rx.clone();
        m[HEADER_LEN + BODY_LEN..HEADER_LEN + BODY_LEN + 4]
            .copy_from_slice(&MACH_PORT_DEAD.to_ne_bytes());
        assert_eq!(parse(&m).unwrap_err(), ParseError::Descriptor);
        // Truncated trailer.
        assert_eq!(parse(&rx[..rx.len() - 1]).unwrap_err(), ParseError::Trailer);
        // An unknown id.
        let mut m = rx.clone();
        m[20..24].copy_from_slice(&0x5331_0004u32.to_ne_bytes());
        assert_eq!(parse(&m).unwrap_err(), ParseError::UnknownId(0x5331_0004));
    }

    /// Deterministic fuzzing: random buffers and mutations of every valid
    /// message kind. The parser must never panic, and anything it accepts must
    /// satisfy the field checks.
    #[test]
    fn fuzz_parser_total() {
        let mut rng = XorShift(0x9e37_79b9_7f4a_7c15);
        let seeds = [
            as_received(&encode_checkin(5, 0x2003, 1), [9; 8]),
            as_received(&encode_checkin_reply(5, 0x2003, 0), [9; 8]),
            as_received(&encode_surface(5, 0x1003, &sample_surface()), [9; 8]),
            as_received(&encode_retire(5, 1, 2), [9; 8]),
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
                if let Incoming::Surface { info, .. } = msg {
                    assert!(validate_surface(&info).is_ok());
                }
            }
        }
        // Mutations that only touch payload values are legitimately accepted.
        assert!(accepted > 0);
    }

    pub(crate) struct XorShift(pub u64);

    impl XorShift {
        pub fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
    }
}
