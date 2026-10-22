//! Opaque WebM packet mux matching legacy/core/exporter.ts `muxWebM`.
//!
//! FFmpeg 9.0.2's Matroska writer unconditionally emits Opus SeekPreRoll,
//! while the current exporter emits neither it nor CodecDelay and writes only
//! SimpleBlocks. Demux the just-encoded packets, then write today's EBML
//! track and cluster structure without changing codec bytes.

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::ptr;

use pm_media::sys::ffmpeg as ff;

use super::{Demuxer, MS, check, cstr};
use crate::Error;
use crate::settings::{Codec, ExportSpec};

const CLUSTER: u32 = 0x1f43b675;

struct Block {
    ms: u64,
    track: u8,
    key: bool,
    data: Vec<u8>,
}

struct AudioTrack {
    private: Vec<u8>,
    rate: u32,
    channels: u8,
}

fn vint_len(n: usize) -> usize {
    let mut len = 1;
    while n as u128 >= (1_u128 << (7 * len)) - 1 {
        len += 1;
    }
    len
}

fn vint(n: usize) -> Vec<u8> {
    let len = vint_len(n);
    let mut bytes = vec![0; len];
    let mut value = n;
    for byte in bytes.iter_mut().rev() {
        *byte = value as u8;
        value >>= 8;
    }
    bytes[0] |= 1 << (8 - len);
    bytes
}

fn uint_bytes(n: u64) -> Vec<u8> {
    if n == 0 {
        return vec![0];
    }
    let bytes = n.to_be_bytes();
    bytes[bytes.iter().position(|b| *b != 0).unwrap()..].to_vec()
}

fn id_bytes(id: u32) -> Vec<u8> {
    uint_bytes(u64::from(id))
}

fn element(id: u32, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(id_bytes(id).len() + vint_len(data.len()) + data.len());
    out.extend_from_slice(&id_bytes(id));
    out.extend_from_slice(&vint(data.len()));
    out.extend_from_slice(data);
    out
}

fn elements(id: u32, parts: &[Vec<u8>]) -> Vec<u8> {
    let data: Vec<u8> = parts.iter().flat_map(|p| p.iter().copied()).collect();
    element(id, &data)
}

fn uint_element(id: u32, n: u64) -> Vec<u8> {
    element(id, &uint_bytes(n))
}

fn size_of_element(id: u32, payload: usize) -> usize {
    id_bytes(id).len() + vint_len(payload) + payload
}

fn write_element_header(file: &mut File, id: u32, payload: usize) -> Result<(), Error> {
    file.write_all(&id_bytes(id))
        .and_then(|()| file.write_all(&vint(payload)))
        .map_err(|e| Error::Io(format!("write legacy WebM element: {e}")))
}

fn packets(path: &Path) -> Result<(Vec<Block>, Option<AudioTrack>), Error> {
    let p = cstr(&path.to_string_lossy());
    let mut fmt: *mut ff::AVFormatContext = ptr::null_mut();
    unsafe {
        check(
            "open encoded WebM for legacy mux",
            ff::avformat_open_input(&mut fmt, p.as_ptr(), ptr::null(), ptr::null_mut()),
        )?;
    }
    let input = Demuxer(fmt);
    unsafe {
        check(
            "read encoded WebM for legacy mux",
            ff::avformat_find_stream_info(input.0, ptr::null_mut()),
        )?;
    }
    let streams: Vec<*mut ff::AVStream> = unsafe {
        (0..(*input.0).nb_streams as usize)
            .map(|i| *(*input.0).streams.add(i))
            .collect()
    };
    let find_stream = |kind| {
        streams
            .iter()
            .enumerate()
            .find_map(|(index, stream)| unsafe {
                if (**stream).codecpar.as_ref()?.codec_type == kind {
                    Some(index)
                } else {
                    None
                }
            })
    };
    let video = find_stream(ff::AVMEDIA_TYPE_VIDEO)
        .ok_or_else(|| Error::Mux("encoded WebM has no video packets".into()))?;
    let audio = find_stream(ff::AVMEDIA_TYPE_AUDIO);
    let audio_info = audio.map(|index| unsafe {
        let par = &*(*streams[index]).codecpar;
        let private = if par.extradata_size > 0 && !par.extradata.is_null() {
            std::slice::from_raw_parts(par.extradata, par.extradata_size as usize).to_vec()
        } else {
            vec![]
        };
        AudioTrack {
            private,
            rate: par.sample_rate as u32,
            channels: par.ch_layout.nb_channels as u8,
        }
    });
    let mut packet = unsafe { ff::av_packet_alloc() };
    if packet.is_null() {
        return Err(Error::Mux("allocate legacy WebM packet".into()));
    }
    let result = (|| {
        let mut blocks = Vec::new();
        loop {
            let rc = unsafe { ff::av_read_frame(input.0, packet) };
            if rc == ff::AVERROR_EOF {
                break;
            }
            check("read legacy WebM packet", rc)?;
            let index = unsafe { (*packet).stream_index as usize };
            let track = if index == video {
                1
            } else if Some(index) == audio {
                2
            } else {
                unsafe { ff::av_packet_unref(packet) };
                continue;
            };
            let block = unsafe {
                let pts = ff::av_rescale_q((*packet).pts, (*streams[index]).time_base, MS);
                if pts < 0 {
                    ff::av_packet_unref(packet);
                    return Err(Error::Mux("negative legacy WebM packet timestamp".into()));
                }
                Block {
                    ms: pts as u64,
                    track,
                    key: track == 1 && ((*packet).flags & ff::AV_PKT_FLAG_KEY as i32) != 0,
                    data: std::slice::from_raw_parts((*packet).data, (*packet).size as usize)
                        .to_vec(),
                }
            };
            unsafe { ff::av_packet_unref(packet) };
            blocks.push(block);
        }
        // JS Array#sort is stable: equal timestamps put video before audio.
        blocks.sort_by_key(|b| (b.ms, b.track));
        Ok(blocks)
    })();
    unsafe { ff::av_packet_free(&mut packet) };
    result.map(|blocks| (blocks, audio_info))
}

pub(super) fn remux(path: &Path, spec: &ExportSpec) -> Result<(), Error> {
    let (blocks, audio_info) = packets(path)?;
    let codec = match spec.codec {
        Codec::Vp9 { .. } => "V_VP9",
        Codec::Vp8 { .. } => "V_VP8",
        _ => return Err(Error::Mux("legacy WebM needs VP9 or VP8".into())),
    };
    let header = elements(
        0x1a45dfa3,
        &[
            uint_element(0x4286, 1),
            uint_element(0x42f7, 1),
            uint_element(0x42f2, 4),
            uint_element(0x42f3, 8),
            element(0x4282, b"webm"),
            uint_element(0x4287, 2),
            uint_element(0x4285, 2),
        ],
    );
    let last_ms = blocks.last().map_or(0, |b| b.ms);
    let duration_ms = last_ms as f64 + 1000.0 / spec.rate.to_f64();
    let info = elements(
        0x1549a966,
        &[
            uint_element(0x2ad7b1, 1_000_000),
            element(0x4d80, b"Powermove"),
            element(0x5741, b"Powermove 1.0.0"),
            element(0x4489, &duration_ms.to_be_bytes()),
        ],
    );
    let mut tracks = vec![elements(
        0xae,
        &[
            uint_element(0xd7, 1),
            uint_element(0x73c5, 1),
            uint_element(0x83, 1),
            element(0x536e, b"Video"),
            element(0x86, codec.as_bytes()),
            elements(
                0xe0,
                &[
                    uint_element(0xb0, u64::from(spec.width)),
                    uint_element(0xba, u64::from(spec.height)),
                ],
            ),
        ],
    )];
    if let Some(AudioTrack {
        private,
        rate,
        channels,
    }) = audio_info.filter(|_| blocks.iter().any(|b| b.track == 2))
    {
        let mut parts = vec![
            uint_element(0xd7, 2),
            uint_element(0x73c5, 2),
            uint_element(0x83, 2),
            element(0x536e, b"Audio"),
            element(0x86, b"A_OPUS"),
        ];
        if !private.is_empty() {
            parts.push(element(0x63a2, &private));
        }
        parts.push(elements(
            0xe1,
            &[
                element(0xb5, &(rate as f64).to_be_bytes()),
                uint_element(0x9f, u64::from(channels)),
            ],
        ));
        tracks.push(elements(0xae, &parts));
    }
    let tracks = elements(0x1654ae6b, &tracks);

    let mut groups: Vec<(u64, Vec<&Block>)> = Vec::new();
    for block in &blocks {
        if groups
            .last()
            .is_none_or(|(start, _)| block.ms - *start > 2000)
        {
            groups.push((block.ms, Vec::new()));
        }
        groups.last_mut().unwrap().1.push(block);
    }
    let cluster_size = |start: u64, group: &[&Block]| {
        let payload = uint_element(0xe7, start).len()
            + group
                .iter()
                .map(|b| size_of_element(0xa3, 4 + b.data.len()))
                .sum::<usize>();
        (payload, size_of_element(CLUSTER, payload))
    };
    let segment_size = info.len()
        + tracks.len()
        + groups
            .iter()
            .map(|(start, group)| cluster_size(*start, group).1)
            .sum::<usize>();
    let mut file = File::create(path)
        .map_err(|e| Error::Io(format!("replace encoded WebM with legacy mux: {e}")))?;
    file.write_all(&header)
        .map_err(|e| Error::Io(format!("write legacy WebM header: {e}")))?;
    write_element_header(&mut file, 0x18538067, segment_size)?;
    file.write_all(&info)
        .and_then(|()| file.write_all(&tracks))
        .map_err(|e| Error::Io(format!("write legacy WebM tracks: {e}")))?;
    for (start, group) in groups {
        let (payload, _) = cluster_size(start, &group);
        write_element_header(&mut file, CLUSTER, payload)?;
        file.write_all(&uint_element(0xe7, start))
            .map_err(|e| Error::Io(format!("write legacy WebM cluster time: {e}")))?;
        for block in group {
            let relative = block.ms - start;
            let mut data = Vec::with_capacity(4 + block.data.len());
            data.push(0x80 | block.track);
            data.extend_from_slice(&(relative as u16).to_be_bytes());
            data.push(if block.key { 0x80 } else { 0 });
            data.extend_from_slice(&block.data);
            file.write_all(&element(0xa3, &data))
                .map_err(|e| Error::Io(format!("write legacy WebM block: {e}")))?;
        }
    }
    file.sync_all()
        .map_err(|e| Error::Io(format!("sync legacy WebM: {e}")))
}
