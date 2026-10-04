//! Resumable downloads and archive extraction.

use crate::{Error, Progress, Result};
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

/// Downloads `url` to `dest`, resuming a previous partial download if one
/// exists. When `sha256` is given the finished file must match it.
pub async fn download(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    sha256: Option<&str>,
    progress: Progress<'_>,
) -> Result<()> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let part = part_path(dest);
    let mut have = tokio::fs::metadata(&part)
        .await
        .map(|m| m.len())
        .unwrap_or(0);

    let mut req = client.get(url);
    if have > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let res = req.send().await?.error_for_status()?;

    // A server that ignores Range sends the whole file again.
    let resumed = res.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    if !resumed {
        have = 0;
    }
    let total = res.content_length().map(|len| len + have);

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(&part)
        .await?;

    let mut done = have;
    progress(done, total);
    let mut stream = res.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        done += chunk.len() as u64;
        progress(done, total);
    }
    file.flush().await?;
    drop(file);

    if let Some(expected) = sha256 {
        let actual = sha256_file(&part).await?;
        if !actual.eq_ignore_ascii_case(expected) {
            tokio::fs::remove_file(&part).await?;
            return Err(Error::other(format!(
                "checksum mismatch for {url}: expected {expected}, got {actual}"
            )));
        }
    }
    tokio::fs::rename(&part, dest).await?;
    Ok(())
}

fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    dest.with_file_name(name)
}

pub async fn sha256_file(path: &Path) -> Result<String> {
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || -> Result<String> {
        let mut file = std::fs::File::open(path)?;
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher)?;
        Ok(hex(&hasher.finalize()))
    })
    .await
    .map_err(|e| Error::other(e.to_string()))?
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Extracts a `.zip` or `.tar.gz` archive into `dest`.
pub async fn extract(archive: &Path, dest: &Path) -> Result<()> {
    let archive = archive.to_owned();
    let dest = dest.to_owned();
    tokio::task::spawn_blocking(move || -> Result<()> {
        std::fs::create_dir_all(&dest)?;
        let file = std::fs::File::open(&archive)?;
        let name = archive.to_string_lossy();
        if name.ends_with(".zip") {
            zip::ZipArchive::new(file)?.extract(&dest)?;
        } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
            tar::Archive::new(flate2::read::GzDecoder::new(file)).unpack(&dest)?;
        } else {
            return Err(Error::other(format!("unknown archive type: {name}")));
        }
        Ok(())
    })
    .await
    .map_err(|e| Error::other(e.to_string()))?
}

/// Finds a file by name anywhere under `dir`.
pub fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            subdirs.push(path);
        } else if path.file_name().is_some_and(|n| n == name) {
            return Some(path);
        }
    }
    subdirs.iter().find_map(|d| find_file(d, name))
}
