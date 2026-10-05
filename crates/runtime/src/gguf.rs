//! Reads just enough of a GGUF file's header to tell a model you can chat
//! with from a speculative decoding draft (EAGLE-3, DFlash, DSpark, Gemma 4
//! assistant). Drafts are made for one main model and can't run alone:
//! llama-server fails with "requires ctx_other to be set".

use crate::{Error, Result};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

/// A draft model that speeds up the main model it was trained for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftKind {
    Eagle3,
    DFlash,
    DSpark,
    /// Gemma 4 assistant, run as a multi-token prediction head.
    Mtp,
}

impl DraftKind {
    /// The `--spec-type` value llama-server expects for this draft.
    pub fn spec_type(self) -> &'static str {
        match self {
            DraftKind::Eagle3 => "draft-eagle3",
            DraftKind::DFlash => "draft-dflash",
            DraftKind::DSpark => "draft-dspark",
            DraftKind::Mtp => "draft-mtp",
        }
    }

    pub fn from_spec_type(s: &str) -> Option<Self> {
        [
            DraftKind::Eagle3,
            DraftKind::DFlash,
            DraftKind::DSpark,
            DraftKind::Mtp,
        ]
        .into_iter()
        .find(|k| k.spec_type() == s)
    }

    /// A name people may recognise from the model page.
    pub fn label(self) -> &'static str {
        match self {
            DraftKind::Eagle3 => "EAGLE-3",
            DraftKind::DFlash => "DFlash",
            DraftKind::DSpark => "DSpark",
            DraftKind::Mtp => "Gemma 4 assistant",
        }
    }
}

const MAGIC: &[u8; 4] = b"GGUF";

/// What kind of draft the file is, or None for an ordinary model.
pub fn draft_kind(path: &Path) -> Result<Option<DraftKind>> {
    let mut r = BufReader::with_capacity(1 << 16, File::open(path)?);
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(Error::other(format!(
            "{} is not a GGUF file",
            path.display()
        )));
    }
    let version = read_u32(&mut r)?;
    if version < 2 {
        // Version 1 files predate drafts entirely.
        return Ok(None);
    }
    let tensors = read_u64(&mut r)?;
    let kvs = read_u64(&mut r)?;
    let mut arch = None;
    for _ in 0..kvs {
        let key = read_string(&mut r)?;
        let ty = read_u32(&mut r)?;
        if key == "general.architecture" && ty == STRING {
            arch = Some(read_string(&mut r)?);
        } else {
            skip_value(&mut r, ty)?;
        }
    }
    let kind = match arch.as_deref() {
        Some("eagle3") => DraftKind::Eagle3,
        Some("gemma4-assistant") => DraftKind::Mtp,
        Some("dflash") => DraftKind::DFlash,
        _ => return Ok(None),
    };
    if kind == DraftKind::DFlash {
        // DSpark is a DFlash backbone with an extra Markov head.
        for _ in 0..tensors {
            let name = read_string(&mut r)?;
            let dims = read_u32(&mut r)?;
            skip(&mut r, dims as u64 * 8 + 4 + 8)?;
            if name.starts_with("markov_w1") {
                return Ok(Some(DraftKind::DSpark));
            }
        }
    }
    Ok(Some(kind))
}

const STRING: u32 = 8;
const ARRAY: u32 = 9;

fn scalar_size(ty: u32) -> Option<u64> {
    Some(match ty {
        0 | 1 | 7 => 1,
        2 | 3 => 2,
        4..=6 => 4,
        10..=12 => 8,
        _ => return None,
    })
}

fn skip_value(r: &mut BufReader<File>, ty: u32) -> Result<()> {
    if let Some(n) = scalar_size(ty) {
        return skip(r, n);
    }
    match ty {
        STRING => {
            let len = read_u64(r)?;
            skip(r, len)
        }
        ARRAY => {
            let inner = read_u32(r)?;
            let count = read_u64(r)?;
            if let Some(n) = scalar_size(inner) {
                return skip(r, n.saturating_mul(count));
            }
            for _ in 0..count {
                skip_value(r, inner)?;
            }
            Ok(())
        }
        _ => Err(Error::other(format!("unknown GGUF value type {ty}"))),
    }
}

fn skip(r: &mut BufReader<File>, n: u64) -> Result<()> {
    let n = i64::try_from(n).map_err(|_| Error::other("GGUF header is damaged"))?;
    r.seek_relative(n)?;
    Ok(())
}

fn read_u32(r: &mut impl Read) -> Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn read_u64(r: &mut impl Read) -> Result<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}

fn read_string(r: &mut impl Read) -> Result<String> {
    let len = read_u64(r)?;
    if len > 1 << 20 {
        return Err(Error::other("GGUF header is damaged"));
    }
    let mut b = vec![0u8; len as usize];
    r.read_exact(&mut b)?;
    Ok(String::from_utf8_lossy(&b).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string(out: &mut Vec<u8>, s: &str) {
        out.extend((s.len() as u64).to_le_bytes());
        out.extend(s.as_bytes());
    }

    /// A tiny GGUF header with a token list, an architecture and tensor names.
    fn header(arch: &str, tensors: &[&str]) -> Vec<u8> {
        let mut out = b"GGUF".to_vec();
        out.extend(3u32.to_le_bytes());
        out.extend((tensors.len() as u64).to_le_bytes());
        out.extend(3u64.to_le_bytes());
        // A string array, as the tokenizer has, before the architecture.
        string(&mut out, "tokenizer.ggml.tokens");
        out.extend(ARRAY.to_le_bytes());
        out.extend(STRING.to_le_bytes());
        out.extend(2u64.to_le_bytes());
        string(&mut out, "<s>");
        string(&mut out, "hello");
        string(&mut out, "general.architecture");
        out.extend(STRING.to_le_bytes());
        string(&mut out, arch);
        string(&mut out, "dflash.block_size");
        out.extend(4u32.to_le_bytes());
        out.extend(16u32.to_le_bytes());
        for t in tensors {
            string(&mut out, t);
            out.extend(2u32.to_le_bytes());
            out.extend(64u64.to_le_bytes());
            out.extend(64u64.to_le_bytes());
            out.extend(0u32.to_le_bytes());
            out.extend(0u64.to_le_bytes());
        }
        out
    }

    fn kind_of(bytes: &[u8]) -> Result<Option<DraftKind>> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.gguf");
        std::fs::write(&path, bytes).unwrap();
        draft_kind(&path)
    }

    #[test]
    fn tells_drafts_from_models() {
        assert_eq!(
            kind_of(&header("qwen3", &["token_embd.weight"])).unwrap(),
            None
        );
        assert_eq!(
            kind_of(&header("dflash", &["blk.0.attn_q.weight"])).unwrap(),
            Some(DraftKind::DFlash)
        );
        assert_eq!(
            kind_of(&header(
                "dflash",
                &["blk.0.attn_q.weight", "markov_w1.weight"]
            ))
            .unwrap(),
            Some(DraftKind::DSpark)
        );
        assert_eq!(
            kind_of(&header("eagle3", &[])).unwrap(),
            Some(DraftKind::Eagle3)
        );
        assert_eq!(
            kind_of(&header("gemma4-assistant", &[])).unwrap(),
            Some(DraftKind::Mtp)
        );
        assert!(kind_of(b"nope").is_err());
        for k in [
            DraftKind::Eagle3,
            DraftKind::DFlash,
            DraftKind::DSpark,
            DraftKind::Mtp,
        ] {
            assert_eq!(DraftKind::from_spec_type(k.spec_type()), Some(k));
        }
    }
}
