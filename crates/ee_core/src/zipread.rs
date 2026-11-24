//! Minimal ZIP reader (central directory; stored + deflate via miniz_oxide), size-limited.
use crate::sexpr::{err, ParseError};

pub const MAX_ENTRY: usize = 64 << 20;
pub const MAX_TOTAL: usize = 256 << 20;

fn u16le(b: &[u8], i: usize) -> Option<usize> {
    Some(u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?]) as usize)
}
fn u32le(b: &[u8], i: usize) -> Option<usize> {
    Some(u32::from_le_bytes([*b.get(i)?, *b.get(i + 1)?, *b.get(i + 2)?, *b.get(i + 3)?]) as usize)
}

pub fn is_zip(b: &[u8]) -> bool {
    b.starts_with(b"PK\x03\x04") || b.starts_with(b"PK\x05\x06")
}

/// All file entries as (name, bytes). Directories are skipped.
pub fn read(b: &[u8]) -> Result<Vec<(String, Vec<u8>)>, ParseError> {
    let bad = |m: &str| ParseError { line: 0, msg: format!("zip: {m}") };
    // End of central directory: scan back over a possible comment.
    let lo = b.len().saturating_sub(22 + 65_535);
    let eocd = (lo..=b.len().saturating_sub(22)).rev().find(|&i| b.get(i..i + 4) == Some(b"PK\x05\x06")).ok_or_else(|| bad("no end of central directory"))?;
    let count = u16le(b, eocd + 10).ok_or_else(|| bad("truncated"))?;
    let mut p = u32le(b, eocd + 16).ok_or_else(|| bad("truncated"))?;
    let mut out = Vec::new();
    let mut total = 0usize;
    for _ in 0..count {
        if b.get(p..p + 4) != Some(b"PK\x01\x02") {
            return Err(bad("bad central directory entry"));
        }
        let field = |o: usize| u16le(b, p + o).ok_or_else(|| bad("truncated"));
        let method = field(10)?;
        let csize = u32le(b, p + 20).ok_or_else(|| bad("truncated"))?;
        let usize_ = u32le(b, p + 24).ok_or_else(|| bad("truncated"))?;
        let (nlen, xlen, clen) = (field(28)?, field(30)?, field(32)?);
        let local = u32le(b, p + 42).ok_or_else(|| bad("truncated"))?;
        let name = String::from_utf8_lossy(b.get(p + 46..p + 46 + nlen).ok_or_else(|| bad("truncated name"))?).into_owned();
        p += 46 + nlen + xlen + clen;
        if name.ends_with('/') {
            continue;
        }
        if b.get(local..local + 4) != Some(b"PK\x03\x04") {
            return Err(bad("bad local header"));
        }
        let data_at = local + 30 + u16le(b, local + 26).ok_or_else(|| bad("truncated"))? + u16le(b, local + 28).ok_or_else(|| bad("truncated"))?;
        let data = b.get(data_at..data_at + csize).ok_or_else(|| bad("truncated data"))?;
        if usize_ > MAX_ENTRY || total + usize_ > MAX_TOTAL {
            return Err(bad("archive too large"));
        }
        let bytes = match method {
            0 => data.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(data, MAX_ENTRY).map_err(|_| bad("deflate error"))?,
            m => return err(0, format!("zip: unsupported compression method {m} in {name}")),
        };
        total += bytes.len();
        out.push((name, bytes));
    }
    Ok(out)
}

#[cfg(test)]
pub mod tests {
    /// Build a zip (method 8 for every entry) for tests. CRC is left 0: the reader ignores it.
    pub fn make_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut cd = Vec::new();
        for (name, data) in files {
            let comp = miniz_oxide::deflate::compress_to_vec(data, 6);
            let off = out.len() as u32;
            let hdr = |sig: &[u8], central: bool, buf: &mut Vec<u8>| {
                buf.extend_from_slice(sig);
                if central {
                    buf.extend_from_slice(&20u16.to_le_bytes());
                }
                buf.extend_from_slice(&20u16.to_le_bytes());
                buf.extend_from_slice(&0u16.to_le_bytes());
                buf.extend_from_slice(&8u16.to_le_bytes());
                buf.extend_from_slice(&[0; 4]);
                buf.extend_from_slice(&0u32.to_le_bytes());
                buf.extend_from_slice(&(comp.len() as u32).to_le_bytes());
                buf.extend_from_slice(&(data.len() as u32).to_le_bytes());
                buf.extend_from_slice(&(name.len() as u16).to_le_bytes());
                buf.extend_from_slice(&0u16.to_le_bytes());
                if central {
                    buf.extend_from_slice(&[0; 6]);
                    buf.extend_from_slice(&0u32.to_le_bytes());
                    buf.extend_from_slice(&off.to_le_bytes());
                }
                buf.extend_from_slice(name.as_bytes());
            };
            hdr(b"PK\x03\x04", false, &mut out);
            out.extend_from_slice(&comp);
            hdr(b"PK\x01\x02", true, &mut cd);
        }
        let cd_off = out.len() as u32;
        out.extend_from_slice(&cd);
        out.extend_from_slice(b"PK\x05\x06");
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(cd.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_off.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    #[test]
    fn reads_deflated_entries_and_rejects_garbage() {
        let z = make_zip(&[("a.gtl", b"hello hello hello"), ("dir/b.drl", b"M48\n")]);
        assert!(super::is_zip(&z));
        let files = super::read(&z).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0], ("a.gtl".to_string(), b"hello hello hello".to_vec()));
        assert_eq!(files[1].0, "dir/b.drl");
        assert!(super::read(b"PK\x03\x04garbage").is_err());
        for cut in [10, z.len() / 2, z.len() - 5] {
            let _ = super::read(&z[..cut]);
        }
    }
}
