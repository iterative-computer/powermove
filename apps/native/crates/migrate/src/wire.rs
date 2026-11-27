use anyhow::{Result, bail, ensure};

pub(crate) const MAX_VALUE: usize = 64 * 1024 * 1024;

pub(crate) struct Cursor<'a>(pub &'a [u8]);
impl<'a> Cursor<'a> {
    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        ensure!(n <= self.0.len(), "truncated encoding");
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    pub fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn var(&mut self) -> Result<u64> {
        let mut n = 0;
        for shift in (0..70).step_by(7) {
            let b = self.byte()?;
            ensure!(shift != 63 || b <= 1, "varint overflow");
            n |= u64::from(b & 127) << shift;
            if b < 128 {
                return Ok(n);
            }
        }
        bail!("varint overflow")
    }
    pub fn len(&mut self) -> Result<usize> {
        let n = usize::try_from(self.var()?)?;
        ensure!(n <= MAX_VALUE, "metadata exceeds the 64 MiB safety bound");
        Ok(n)
    }
    pub fn bytes(&mut self) -> Result<&'a [u8]> {
        let n = self.len()?;
        self.take(n)
    }
    pub fn utf16(&mut self) -> Result<String> {
        let n = self
            .len()?
            .checked_mul(2)
            .ok_or_else(|| anyhow::anyhow!("length overflow"))?;
        utf16(self.take(n)?, true)
    }
    pub fn finish(&self) -> Result<()> {
        ensure!(self.0.is_empty(), "unexpected trailing bytes");
        Ok(())
    }
}

pub(crate) fn utf16(bytes: &[u8], be: bool) -> Result<String> {
    ensure!(bytes.len().is_multiple_of(2), "odd UTF-16 length");
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|v| {
            if be {
                u16::from_be_bytes([v[0], v[1]])
            } else {
                u16::from_le_bytes([v[0], v[1]])
            }
        })
        .collect();
    String::from_utf16(&units).map_err(|_| anyhow::anyhow!("invalid UTF-16"))
}

pub(crate) fn unmask(crc: u32) -> u32 {
    crc.wrapping_sub(0xa282ead8).rotate_left(15)
}
