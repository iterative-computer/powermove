//! sfnt assembly shared by the subsetter, the web font sanitizer model, and
//! the registry (fonts CoreText serves only as tables).

/// Assembles a single-face sfnt from (tag, data) pairs. Tables are sorted by
/// tag and padded to four bytes; checksums are computed per table. The
/// `head` checksum adjustment is left as the font had it: no consumer of
/// these files (HarfRust, read-fonts, Parley) validates it.
pub fn build_sfnt(tables: &mut [(u32, Vec<u8>)]) -> Vec<u8> {
    tables.sort_by_key(|t| t.0);
    let n = tables.len() as u16;
    let mut entry_selector = 0u16;
    while (1u32 << (entry_selector + 1)) <= n as u32 {
        entry_selector += 1;
    }
    let search_range = if n == 0 {
        0
    } else {
        (1u16 << entry_selector) * 16
    };
    let range_shift = (n * 16).saturating_sub(search_range);
    let is_cff = tables
        .iter()
        .any(|t| t.0 == u32::from_be_bytes(*b"CFF ") || t.0 == u32::from_be_bytes(*b"CFF2"));
    let mut out = Vec::with_capacity(
        12 + 16 * tables.len() + tables.iter().map(|t| t.1.len() + 3).sum::<usize>(),
    );
    out.extend_from_slice(
        &(if is_cff {
            0x4F54_544Fu32
        } else {
            0x0001_0000u32
        })
        .to_be_bytes(),
    );
    out.extend_from_slice(&n.to_be_bytes());
    out.extend_from_slice(&search_range.to_be_bytes());
    out.extend_from_slice(&entry_selector.to_be_bytes());
    out.extend_from_slice(&range_shift.to_be_bytes());
    let mut offset = 12 + 16 * tables.len();
    let mut body = Vec::new();
    for (tag, data) in tables.iter() {
        out.extend_from_slice(&tag.to_be_bytes());
        out.extend_from_slice(&checksum(data).to_be_bytes());
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        body.extend_from_slice(data);
        let pad = (4 - data.len() % 4) % 4;
        body.extend(std::iter::repeat_n(0u8, pad));
        offset += data.len() + pad;
    }
    out.extend_from_slice(&body);
    out
}

fn checksum(data: &[u8]) -> u32 {
    let mut sum = 0u32;
    for chunk in data.chunks(4) {
        let mut b = [0u8; 4];
        b[..chunk.len()].copy_from_slice(chunk);
        sum = sum.wrapping_add(u32::from_be_bytes(b));
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;
    use read_fonts::{FontRef, TableProvider};

    #[test]
    fn round_trips_through_read_fonts() {
        let mut head = vec![0u8; 54];
        head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
        head[18..20].copy_from_slice(&1000u16.to_be_bytes());
        let mut tables = vec![
            (u32::from_be_bytes(*b"zzzz"), vec![1, 2, 3]),
            (u32::from_be_bytes(*b"head"), head),
        ];
        let bytes = build_sfnt(&mut tables);
        let font = FontRef::new(&bytes).unwrap();
        assert_eq!(font.head().unwrap().units_per_em(), 1000);
        assert_eq!(
            font.table_data(read_fonts::types::Tag::new(b"zzzz"))
                .unwrap()
                .as_bytes(),
            &[1, 2, 3]
        );
        assert_eq!(font.table_directory().table_records().len(), 2);
    }
}
