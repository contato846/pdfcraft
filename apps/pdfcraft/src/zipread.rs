//! Reads one file out of a zip archive (the portable build's download): the central directory
//! at the end names the entries, each stored as is or deflated. Untrusted input: every offset
//! and size is checked, and the inflated size is capped.

use std::io::Read;

/// The bytes of entry `name` in `zip`, inflated; at most `max` bytes.
pub fn entry(zip: &[u8], name: &str, max: usize) -> Result<Vec<u8>, String> {
    let (count, dir_offset) = end_of_central_directory(zip)?;
    let mut at = dir_offset;
    for _ in 0..count {
        let header = zip.get(at..at.checked_add(46).ok_or("zip: overflow")?).ok_or("zip: central directory cut off")?;
        if !header.starts_with(b"PK\x01\x02") {
            return Err("zip: bad central directory entry".into());
        }
        let method = u16::from_le_bytes([header[10], header[11]]);
        let compressed = u32::from_le_bytes([header[20], header[21], header[22], header[23]]) as usize;
        let size = u32::from_le_bytes([header[24], header[25], header[26], header[27]]) as usize;
        let name_len = u16::from_le_bytes([header[28], header[29]]) as usize;
        let extra_len = u16::from_le_bytes([header[30], header[31]]) as usize;
        let comment_len = u16::from_le_bytes([header[32], header[33]]) as usize;
        let local = u32::from_le_bytes([header[42], header[43], header[44], header[45]]) as usize;
        let name_at = at + 46;
        let entry_name = zip.get(name_at..name_at.checked_add(name_len).ok_or("zip: overflow")?).ok_or("zip: name cut off")?;
        if entry_name == name.as_bytes() {
            return data(zip, local, method, compressed, size, max);
        }
        at = name_at.checked_add(name_len + extra_len + comment_len).ok_or("zip: overflow")?;
    }
    Err(format!("zip: no entry named {name}"))
}

/// (entry count, central directory offset) from the end record, which is followed by a comment
/// of up to 65 535 bytes.
fn end_of_central_directory(zip: &[u8]) -> Result<(usize, usize), String> {
    let start = zip.len().saturating_sub(22 + 65_535);
    let tail = zip.get(start..).ok_or("zip: empty")?;
    let rel = tail.windows(4).rposition(|w| w == b"PK\x05\x06").ok_or("zip: no end record")?;
    let rec = tail.get(rel..rel + 22).ok_or("zip: end record cut off")?;
    let count = u16::from_le_bytes([rec[10], rec[11]]) as usize;
    let offset = u32::from_le_bytes([rec[16], rec[17], rec[18], rec[19]]) as usize;
    Ok((count, offset))
}

fn data(zip: &[u8], local: usize, method: u16, compressed: usize, size: usize, max: usize) -> Result<Vec<u8>, String> {
    if size > max {
        return Err("zip: the entry is larger than allowed".into());
    }
    let header = zip.get(local..local.checked_add(30).ok_or("zip: overflow")?).ok_or("zip: local header cut off")?;
    if !header.starts_with(b"PK\x03\x04") {
        return Err("zip: bad local header".into());
    }
    let name_len = u16::from_le_bytes([header[26], header[27]]) as usize;
    let extra_len = u16::from_le_bytes([header[28], header[29]]) as usize;
    let start = local.checked_add(30 + name_len + extra_len).ok_or("zip: overflow")?;
    let raw = zip.get(start..start.checked_add(compressed).ok_or("zip: overflow")?).ok_or("zip: entry data cut off")?;
    match method {
        0 => Ok(raw.to_vec()),
        8 => {
            let mut out = Vec::with_capacity(size);
            flate2::read::DeflateDecoder::new(raw).take(size as u64).read_to_end(&mut out).map_err(|e| format!("zip: inflate failed ({e})"))?;
            if out.len() != size {
                return Err("zip: the entry's size does not match".into());
            }
            Ok(out)
        }
        m => Err(format!("zip: compression method {m} is not supported")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A zip of `entries`, each stored or deflated.
    fn zip(entries: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut dir = Vec::new();
        for (name, body, deflate) in entries {
            let data = if *deflate {
                let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(body).unwrap();
                e.finish().unwrap()
            } else {
                body.to_vec()
            };
            let method: u16 = if *deflate { 8 } else { 0 };
            let local = out.len() as u32;
            out.extend_from_slice(b"PK\x03\x04");
            out.extend_from_slice(&[20, 0, 0, 0]);
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&[0; 8]);
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&[0, 0]);
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&data);
            dir.extend_from_slice(b"PK\x01\x02");
            dir.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
            dir.extend_from_slice(&method.to_le_bytes());
            dir.extend_from_slice(&[0; 8]);
            dir.extend_from_slice(&(data.len() as u32).to_le_bytes());
            dir.extend_from_slice(&(body.len() as u32).to_le_bytes());
            dir.extend_from_slice(&(name.len() as u16).to_le_bytes());
            dir.extend_from_slice(&[0; 12]);
            dir.extend_from_slice(&local.to_le_bytes());
            dir.extend_from_slice(name.as_bytes());
        }
        let dir_at = out.len() as u32;
        out.extend_from_slice(&dir);
        out.extend_from_slice(b"PK\x05\x06");
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(dir.len() as u32).to_le_bytes());
        out.extend_from_slice(&dir_at.to_le_bytes());
        out.extend_from_slice(b"\x05\0hello");
        out
    }

    #[test]
    fn stored_and_deflated_entries_are_read_and_bad_input_is_refused() {
        let exe = vec![7u8; 10_000];
        let z = zip(&[("PdfCraft/LICENSE", b"MIT", false), ("PdfCraft/pdfcraft.exe", &exe, true)]);
        assert_eq!(entry(&z, "PdfCraft/pdfcraft.exe", 1 << 20).unwrap(), exe);
        assert_eq!(entry(&z, "PdfCraft/LICENSE", 1 << 20).unwrap(), b"MIT");
        assert!(entry(&z, "PdfCraft/other", 1 << 20).is_err());
        assert!(entry(&z, "PdfCraft/pdfcraft.exe", 100).is_err(), "over the cap");
        assert!(entry(&z[..z.len() - 30], "PdfCraft/LICENSE", 1 << 20).is_err(), "cut off");
        assert!(entry(b"", "x", 10).is_err());
        assert!(entry(b"PK\x05\x06", "x", 10).is_err());
    }
}
