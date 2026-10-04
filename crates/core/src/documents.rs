//! Turns documents people attach to a chat into text the model can read.

use crate::{Error, Result};
use serde::Serialize;
use std::io::Read;

/// Largest document accepted, before reading.
pub const MAX_DOCUMENT_BYTES: usize = 25 * 1024 * 1024;

/// Most text kept from one document. Small models have small contexts, and
/// the conversation is trimmed to fit anyway.
pub const MAX_DOCUMENT_CHARS: usize = 200_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub text: String,
    /// Pages, for PDFs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages: Option<usize>,
    /// The text was cut at `MAX_DOCUMENT_CHARS`.
    pub truncated: bool,
}

/// Reads the text out of a PDF, a Word document or a text file. `name` picks
/// the format, falling back to the file's first bytes.
pub fn read_document(name: &str, bytes: &[u8]) -> Result<Document> {
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(Error::Invalid(format!(
            "{name} is too big. Attach files up to {} MB.",
            MAX_DOCUMENT_BYTES / 1024 / 1024
        )));
    }
    let lower = name.to_ascii_lowercase();
    let (text, pages) = if lower.ends_with(".pdf") || bytes.starts_with(b"%PDF") {
        read_pdf(name, bytes)?
    } else if lower.ends_with(".docx") {
        (read_docx(name, bytes)?, None)
    } else {
        match std::str::from_utf8(bytes) {
            Ok(text) if !text.contains('\0') => (text.to_string(), None),
            _ => {
                return Err(Error::Invalid(format!(
                    "BrainWashed can't read {name}. Attach a PDF, a Word document, a picture or a text file."
                )))
            }
        }
    };
    let text = tidy(&text);
    if text.trim().is_empty() {
        return Err(Error::Invalid(format!(
            "{name} has no text BrainWashed can read. If it's a scan, attach it as a picture instead."
        )));
    }
    let (text, truncated) = match text.char_indices().nth(MAX_DOCUMENT_CHARS) {
        Some((i, _)) => (text[..i].to_string(), true),
        None => (text, false),
    };
    Ok(Document {
        text,
        pages,
        truncated,
    })
}

fn read_pdf(name: &str, bytes: &[u8]) -> Result<(String, Option<usize>)> {
    // The PDF parser panics on some malformed files; treat that as unreadable.
    let pages = std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem_by_pages(bytes))
        .map_err(|_| Error::Invalid(format!("{name} looks damaged and can't be read.")))?
        .map_err(|e| Error::Invalid(format!("{name} can't be read: {e}")))?;
    let count = pages.len();
    Ok((pages.join("\n\n"), Some(count)))
}

/// The text of a .docx: the `<w:t>` runs of word/document.xml, with a line
/// break per paragraph.
fn read_docx(name: &str, bytes: &[u8]) -> Result<String> {
    let bad = || {
        Error::Invalid(format!(
            "{name} is not a Word document BrainWashed can read."
        ))
    };
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| bad())?;
    let mut xml = String::new();
    zip.by_name("word/document.xml")
        .map_err(|_| bad())?
        .take(50 * 1024 * 1024)
        .read_to_string(&mut xml)
        .map_err(|_| bad())?;

    let mut text = String::new();
    let mut rest = xml.as_str();
    while let Some(start) = rest.find('<') {
        let Some(end) = rest[start..].find('>') else {
            break;
        };
        let tag = &rest[start + 1..start + end];
        let after = &rest[start + end + 1..];
        let name = tag.split([' ', '/']).next().unwrap_or("");
        match name {
            "w:t" if !tag.ends_with('/') => {
                let close = after.find("</w:t>").unwrap_or(after.len());
                text.push_str(&unescape_xml(&after[..close]));
                rest = &after[close..];
                continue;
            }
            "w:tab" => text.push('\t'),
            "w:br" | "w:cr" => text.push('\n'),
            "" if tag == "/w:p" => text.push('\n'),
            _ => {}
        }
        rest = after;
    }
    Ok(text)
}

fn unescape_xml(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Normalizes line endings and collapses runs of blank lines, which PDFs
/// produce a lot of and which cost tokens.
fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank = 0;
    for line in text.replace("\r\n", "\n").replace('\r', "\n").lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_text_files() {
        let doc = read_document("notes.md", b"# Notes\r\n\r\n\r\n\r\nbuy milk  \n").unwrap();
        assert_eq!(doc.text, "# Notes\n\nbuy milk");
        assert_eq!(doc.pages, None);
        assert!(!doc.truncated);
    }

    #[test]
    fn refuses_binary_and_empty_files() {
        assert!(read_document("photo.heic", &[0, 1, 2, 0xff]).is_err());
        assert!(read_document("empty.txt", b"  \n ").is_err());
        assert!(read_document("broken.pdf", b"%PDF-1.4 nonsense").is_err());
    }

    #[test]
    fn cuts_long_documents() {
        let long = "a".repeat(MAX_DOCUMENT_CHARS + 10);
        let doc = read_document("long.txt", long.as_bytes()).unwrap();
        assert!(doc.truncated);
        assert_eq!(doc.text.len(), MAX_DOCUMENT_CHARS);
    }

    /// A one-page PDF saying `text`, with a correct cross-reference table.
    fn tiny_pdf(text: &str) -> Vec<u8> {
        let stream = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
            format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = pdf.len();
        pdf.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for o in offsets {
            pdf.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }

    #[test]
    fn reads_pdfs() {
        let doc = read_document("invoice.pdf", &tiny_pdf("Total due: 42 dollars")).unwrap();
        assert!(doc.text.contains("Total due: 42 dollars"), "{:?}", doc.text);
        assert_eq!(doc.pages, Some(1));
    }

    #[test]
    fn reads_word_documents() {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            zip.start_file(
                "word/document.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(
                br#"<w:document><w:body><w:p><w:r><w:t>Rent is due</w:t></w:r><w:r><w:t xml:space="preserve"> on the 1st &amp; late fees</w:t></w:r></w:p><w:p><w:r><w:t>apply.</w:t></w:r></w:p></w:body></w:document>"#,
            )
            .unwrap();
            zip.finish().unwrap();
        }
        let doc = read_document("lease.docx", buf.get_ref()).unwrap();
        assert_eq!(doc.text, "Rent is due on the 1st & late fees\napply.");
    }
}
