//! Curated models, and resolving a Hugging Face repo to a GGUF file.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogEntry {
    /// Hugging Face repo, e.g. `bartowski/Llama-3.2-3B-Instruct-GGUF`.
    pub repo: String,
    pub name: String,
    pub description: String,
    /// Parameter count in billions, used to match models to hardware.
    pub params_b: f32,
    pub license: String,
    /// Comes with a vision projector, so it can look at pictures.
    #[serde(default)]
    pub vision: bool,
}

/// Models offered in the picker. Any other GGUF repo can be added by name.
pub fn curated() -> Vec<CatalogEntry> {
    serde_json::from_str(include_str!("catalog.json")).expect("catalog.json is valid")
}

/// A downloadable GGUF file in a Hugging Face repo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelFile {
    pub repo: String,
    pub file: String,
    pub size: u64,
    pub sha256: Option<String>,
}

impl ModelFile {
    pub fn url(&self, hf_base: &str) -> String {
        format!("{hf_base}/{}/resolve/main/{}", self.repo, self.file)
    }
}

pub const HF_BASE: &str = "https://huggingface.co";

/// Quantizations in order of preference: good quality at a small size first.
const PREFERRED_QUANTS: &[&str] = &[
    "Q4_K_M", "Q4_K_S", "Q4_0", "Q5_K_M", "Q5_K_S", "Q6_K", "Q8_0", "Q3_K_L", "Q3_K_M", "F16",
];

#[derive(Debug, Deserialize)]
struct TreeEntry {
    #[serde(rename = "type")]
    kind: String,
    path: String,
    #[serde(default)]
    size: u64,
    lfs: Option<Lfs>,
}

#[derive(Debug, Deserialize)]
struct Lfs {
    oid: String,
    size: u64,
}

/// A model file and, for models that can see pictures, its vision projector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelFiles {
    pub model: ModelFile,
    pub projector: Option<ModelFile>,
}

pub async fn resolve(
    client: &reqwest::Client,
    hf_base: &str,
    repo: &str,
    quant: Option<&str>,
) -> Result<ModelFiles> {
    let url = format!("{hf_base}/api/models/{repo}/tree/main");
    let body = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(ModelFiles {
        model: pick_file(repo, &body, quant)?,
        projector: pick_projector(repo, &body)?,
    })
}

/// Projector precision in order of preference: full quality first, since
/// projectors are small and quantizing them hurts more.
const PREFERRED_PROJECTORS: &[&str] = &["f16", "bf16", "q8_0", "f32"];

/// Chooses the vision projector (`mmproj`) in a repo listing, if it has one.
pub fn pick_projector(repo: &str, tree_json: &str) -> Result<Option<ModelFile>> {
    let entries: Vec<TreeEntry> = serde_json::from_str(tree_json)?;
    let projectors: Vec<&TreeEntry> = entries
        .iter()
        .filter(|e| {
            let lower = e.path.to_ascii_lowercase();
            e.kind == "file" && lower.ends_with(".gguf") && lower.contains("mmproj")
        })
        .collect();
    let chosen = PREFERRED_PROJECTORS
        .iter()
        .find_map(|p| {
            projectors
                .iter()
                .find(|e| e.path.to_ascii_lowercase().contains(p))
        })
        .or(projectors.first());
    Ok(chosen.map(|e| ModelFile {
        repo: repo.to_string(),
        file: e.path.clone(),
        size: e.lfs.as_ref().map_or(e.size, |l| l.size),
        sha256: e.lfs.as_ref().map(|l| l.oid.clone()),
    }))
}

/// Chooses a single-file GGUF from a repo listing, preferring `quant` if given.
pub fn pick_file(repo: &str, tree_json: &str, quant: Option<&str>) -> Result<ModelFile> {
    let entries: Vec<TreeEntry> = serde_json::from_str(tree_json)?;
    let candidates: Vec<&TreeEntry> = entries
        .iter()
        .filter(|e| e.kind == "file" && e.path.to_ascii_lowercase().ends_with(".gguf"))
        .filter(|e| {
            let lower = e.path.to_ascii_lowercase();
            // Vision projectors and split files aren't standalone models.
            !lower.contains("mmproj") && !lower.contains("-of-0")
        })
        .collect();

    let wanted: Vec<&str> = match quant {
        Some(q) => vec![q],
        None => PREFERRED_QUANTS.to_vec(),
    };
    let chosen = wanted
        .iter()
        .find_map(|q| {
            let q = q.to_ascii_lowercase();
            candidates
                .iter()
                .find(|e| e.path.to_ascii_lowercase().contains(&q))
        })
        .or(if quant.is_none() {
            candidates.first()
        } else {
            None
        })
        .ok_or_else(|| Error::other(format!("no matching GGUF file in {repo}")))?;

    Ok(ModelFile {
        repo: repo.to_string(),
        file: chosen.path.clone(),
        size: chosen.lfs.as_ref().map_or(chosen.size, |l| l.size),
        sha256: chosen.lfs.as_ref().map(|l| l.oid.clone()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TREE: &str = r#"[
      {"type":"file","path":"README.md","size":100},
      {"type":"file","path":"Model-Q8_0.gguf","size":10,"lfs":{"oid":"aa","size":3000}},
      {"type":"file","path":"Model-Q4_K_M.gguf","size":10,"lfs":{"oid":"bb","size":2000}},
      {"type":"file","path":"mmproj-Model-Q4_K_M.gguf","size":10,"lfs":{"oid":"cc","size":500}},
      {"type":"file","path":"Model-F16-00001-of-00002.gguf","size":10},
      {"type":"directory","path":"old","size":0}
    ]"#;

    #[test]
    fn prefers_q4_k_m() {
        let f = pick_file("me/model", TREE, None).unwrap();
        assert_eq!(f.file, "Model-Q4_K_M.gguf");
        assert_eq!(f.size, 2000);
        assert_eq!(f.sha256.as_deref(), Some("bb"));
        assert_eq!(
            f.url(HF_BASE),
            "https://huggingface.co/me/model/resolve/main/Model-Q4_K_M.gguf"
        );
    }

    #[test]
    fn honours_requested_quant() {
        assert_eq!(
            pick_file("me/model", TREE, Some("q8_0")).unwrap().file,
            "Model-Q8_0.gguf"
        );
        assert!(pick_file("me/model", TREE, Some("Q2_K")).is_err());
    }

    #[test]
    fn finds_the_vision_projector() {
        let p = pick_projector("me/model", TREE).unwrap().unwrap();
        assert_eq!(p.file, "mmproj-Model-Q4_K_M.gguf");
        assert_eq!(p.size, 500);
        let tree = r#"[
          {"type":"file","path":"mmproj-Q8_0.gguf","size":10},
          {"type":"file","path":"mmproj-F16.gguf","size":20}
        ]"#;
        assert_eq!(
            pick_projector("me/model", tree).unwrap().unwrap().file,
            "mmproj-F16.gguf"
        );
        assert_eq!(pick_projector("me/model", "[]").unwrap(), None);
    }

    #[test]
    fn curated_catalog_parses() {
        assert!(!curated().is_empty());
    }
}
