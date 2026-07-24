use crate::config::BucketConfig;
use std::path::{Path, PathBuf};
use tokio::fs;

pub const TMP_SUFFIX: &str = ".stathost-tmp";

#[derive(serde::Serialize)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    pub mtime: u64,
}

pub struct Bucket {
    path: PathBuf,
    config: BucketConfig,
}

impl Bucket {
    pub fn load(path: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let config = BucketConfig::load(&path)?;
        Ok(Self { path, config })
    }

    pub fn validate_token(&self, token: &str) -> bool {
        self.config.auth.token == token
    }

    pub fn resolve_path(&self, file_path: &str) -> Option<PathBuf> {
        let file_path = file_path.trim_start_matches('/');

        if is_protected_path(file_path) {
            return None;
        }

        let full_path = self.path.join(file_path);

        if !is_safe_path(&self.path, &full_path) {
            return None;
        }

        Some(full_path)
    }

    pub async fn list_files(&self) -> Result<Vec<String>, std::io::Error> {
        let mut files = Vec::new();
        collect_files(&self.path, &self.path, &mut files).await?;
        Ok(files.into_iter().map(|(relative, _)| relative).collect())
    }

    pub async fn list_files_detailed(&self) -> Result<Vec<FileEntry>, std::io::Error> {
        let mut files = Vec::new();
        collect_files(&self.path, &self.path, &mut files).await?;

        let mut entries = Vec::with_capacity(files.len());
        for (relative, path) in files {
            let Ok(metadata) = fs::metadata(&path).await else {
                continue;
            };
            let mtime = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            entries.push(FileEntry {
                path: relative,
                size: metadata.len(),
                mtime,
            });
        }
        Ok(entries)
    }
}

fn is_protected_path(path: &str) -> bool {
    let path_lower = path.to_lowercase();
    path_lower == "config.toml"
        || path_lower.starts_with("config.toml/")
        || path_lower.starts_with("_meta/")
        || path_lower == "_meta"
        || path_lower.ends_with(TMP_SUFFIX)
}

fn is_safe_path(base: &Path, full_path: &Path) -> bool {
    let Ok(canonical_base) = base.canonicalize() else {
        return false;
    };

    // Try to canonicalize the full path (works if file exists)
    if let Ok(canonical) = full_path.canonicalize() {
        return canonical.starts_with(&canonical_base);
    }

    // File doesn't exist - find the deepest existing ancestor
    let mut ancestor = full_path.to_path_buf();
    while !ancestor.exists() {
        if !ancestor.pop() {
            return false;
        }
    }

    if let Ok(canonical_ancestor) = ancestor.canonicalize() {
        return canonical_ancestor.starts_with(&canonical_base);
    }

    false
}

async fn collect_files(
    base: &Path,
    current: &Path,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), std::io::Error> {
    let mut entries = fs::read_dir(current).await?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let relative = path
            .strip_prefix(base)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();

        if is_protected_path(&relative) {
            continue;
        }

        if path.is_dir() {
            Box::pin(collect_files(base, &path, files)).await?;
        } else {
            files.push((relative, path));
        }
    }

    Ok(())
}

pub async fn cleanup_temp_files(dir: &Path) -> Result<(), std::io::Error> {
    let mut entries = fs::read_dir(dir).await?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.is_dir() {
            Box::pin(cleanup_temp_files(&path)).await?;
        } else if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(TMP_SUFFIX))
        {
            let _ = fs::remove_file(&path).await;
        }
    }

    Ok(())
}

pub struct BucketManager {
    buckets_dir: PathBuf,
}

impl BucketManager {
    pub fn new(buckets_dir: PathBuf) -> Self {
        Self { buckets_dir }
    }

    pub fn get_bucket(&self, name: &str) -> Option<Bucket> {
        if name.contains("..") || name.contains('/') || name.contains('\\') {
            return None;
        }

        let bucket_path = self.buckets_dir.join(name);
        if !bucket_path.is_dir() {
            return None;
        }

        Bucket::load(bucket_path).ok()
    }
}
