//! Length-delimited JSON + binary. stdout is reserved for packets; logs use stderr.
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

pub const VERSION: u32 = 1;
pub const MAX_JSON: usize = 4 * 1024 * 1024;
pub const MAX_BINARY: usize = 256 * 1024 * 1024;
const MAGIC: &[u8; 4] = b"PMS1";

#[derive(Debug)]
pub struct Packet {
    pub json: serde_json::Value,
    pub binary: Vec<u8>,
}

pub fn read(input: &mut impl Read) -> io::Result<Option<Packet>> {
    let mut header = [0u8; 12];
    // EOF between packets is normal. A partial packet is a protocol error.
    if input.read(&mut header[..1])? == 0 {
        return Ok(None);
    }
    input.read_exact(&mut header[1..])?;
    if &header[..4] != MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid Studio packet magic",
        ));
    }
    let json_len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
    let binary_len = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
    if json_len == 0 || json_len > MAX_JSON || binary_len > MAX_BINARY {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Studio packet exceeds size limit",
        ));
    }
    let mut json = vec![0; json_len];
    let mut binary = vec![0; binary_len];
    input.read_exact(&mut json)?;
    input.read_exact(&mut binary)?;
    Ok(Some(Packet {
        json: serde_json::from_slice(&json)?,
        binary,
    }))
}

pub fn write(output: &mut impl Write, json: &impl Serialize, binary: &[u8]) -> io::Result<()> {
    let json = serde_json::to_vec(json)?;
    if json.len() > MAX_JSON || binary.len() > MAX_BINARY {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Studio response exceeds size limit",
        ));
    }
    output.write_all(MAGIC)?;
    output.write_all(&(json.len() as u32).to_le_bytes())?;
    output.write_all(&(binary.len() as u32).to_le_bytes())?;
    output.write_all(&json)?;
    output.write_all(binary)?;
    output.flush()
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct Range {
    pub offset: usize,
    pub length: usize,
}
impl Range {
    pub fn bytes(self, binary: &[u8]) -> Result<&[u8], String> {
        let end = self
            .offset
            .checked_add(self.length)
            .ok_or("binary range overflow")?;
        binary
            .get(self.offset..end)
            .ok_or_else(|| "binary range is outside packet".into())
    }
    pub fn typed<T: bytemuck::Pod>(self, binary: &[u8]) -> Result<Vec<T>, String> {
        let bytes = self.bytes(binary)?;
        let size = std::mem::size_of::<T>();
        if !bytes.len().is_multiple_of(size) {
            return Err("binary range has invalid element size".into());
        }
        // Packet offsets are not required to be aligned. Never cast borrowed input.
        Ok(bytes
            .chunks_exact(size)
            .map(bytemuck::pod_read_unaligned)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trip_and_eof() {
        let mut bytes = Vec::new();
        write(
            &mut bytes,
            &serde_json::json!({"id":7,"op":"hello"}),
            &[0, 1, 255],
        )
        .unwrap();
        let mut input = bytes.as_slice();
        let p = read(&mut input).unwrap().unwrap();
        assert_eq!(p.json["id"], 7);
        assert_eq!(p.binary, [0, 1, 255]);
        assert!(read(&mut input).unwrap().is_none());
    }
    #[test]
    fn rejects_truncation_magic_and_oversize() {
        assert!(read(&mut &b"PMS1"[..]).is_err());
        let mut h = *b"NOPE\x01\0\0\0\0\0\0\0";
        assert!(read(&mut h.as_slice()).is_err());
        h[..4].copy_from_slice(MAGIC);
        h[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(read(&mut h.as_slice()).is_err());
    }
    #[test]
    fn unaligned_and_overflow_ranges() {
        assert_eq!(
            Range {
                offset: 1,
                length: 4
            }
            .typed::<u32>(&[0, 1, 0, 0, 0])
            .unwrap(),
            [1]
        );
        assert!(
            Range {
                offset: usize::MAX,
                length: 1
            }
            .bytes(&[])
            .is_err()
        );
        assert!(
            Range {
                offset: 0,
                length: 3
            }
            .typed::<u32>(&[0; 3])
            .is_err()
        );
    }
}
