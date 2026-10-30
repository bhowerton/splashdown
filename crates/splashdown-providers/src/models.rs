//! Downloadable whisper.cpp models (spec §5.2). Downloading doesn't need the
//! `whisper-local` feature, so the app can fetch a model before loading it.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use splashdown_core::ProviderError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhisperModel {
    /// English-only, fast. ~148 MB.
    BaseEn,
    /// English-only, better accuracy. ~488 MB.
    SmallEn,
    /// Multilingual, best accuracy, quantized. ~574 MB.
    LargeV3TurboQ5,
}

impl WhisperModel {
    pub const ALL: [WhisperModel; 3] = [
        WhisperModel::BaseEn,
        WhisperModel::SmallEn,
        WhisperModel::LargeV3TurboQ5,
    ];

    pub fn id(self) -> &'static str {
        match self {
            WhisperModel::BaseEn => "base.en",
            WhisperModel::SmallEn => "small.en",
            WhisperModel::LargeV3TurboQ5 => "large-v3-turbo-q5_0",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id() == id)
    }

    pub fn file_name(self) -> String {
        format!("ggml-{}.bin", self.id())
    }

    pub fn url(self) -> String {
        format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
            self.file_name()
        )
    }

    /// Approximate download size in MB, for UI.
    pub fn approx_mb(self) -> u32 {
        match self {
            WhisperModel::BaseEn => 148,
            WhisperModel::SmallEn => 488,
            WhisperModel::LargeV3TurboQ5 => 574,
        }
    }

    pub fn path_in(self, models_dir: &Path) -> PathBuf {
        models_dir.join(self.file_name())
    }

    /// Whether the model is already downloaded to `models_dir`.
    pub fn is_downloaded(self, models_dir: &Path) -> bool {
        self.path_in(models_dir).is_file()
    }

    /// Downloads the model into `models_dir`. `progress` receives
    /// `(downloaded_bytes, total_bytes)`.
    pub async fn download(
        self,
        models_dir: &Path,
        progress: impl FnMut(u64, Option<u64>),
    ) -> Result<PathBuf, ProviderError> {
        download_file(&self.url(), &self.path_in(models_dir), progress).await
    }
}

/// Built-in local text models: GGUF files from Hugging Face, run by
/// llama.cpp (feature `llm-local`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalModel {
    /// Qwen3 4B Instruct (2507), 4-bit. Apache-2.0. ~2.5 GB.
    Qwen3_4B,
    /// Qwen3 1.7B, 4-bit, thinking off (by `splashdown-llm`). Apache-2.0.
    /// ~1.1 GB. For machines with about 8 GB of memory.
    Qwen3_1_7B,
}

impl LocalModel {
    pub const ALL: [LocalModel; 2] = [LocalModel::Qwen3_4B, LocalModel::Qwen3_1_7B];

    pub fn id(self) -> &'static str {
        match self {
            LocalModel::Qwen3_4B => "qwen3-4b",
            LocalModel::Qwen3_1_7B => "qwen3-1.7b",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id() == id)
    }

    pub fn file_name(self) -> &'static str {
        match self {
            LocalModel::Qwen3_4B => "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
            LocalModel::Qwen3_1_7B => "Qwen3-1.7B-Q4_K_M.gguf",
        }
    }

    pub fn url(self) -> String {
        let repo = match self {
            LocalModel::Qwen3_4B => "unsloth/Qwen3-4B-Instruct-2507-GGUF",
            LocalModel::Qwen3_1_7B => "unsloth/Qwen3-1.7B-GGUF",
        };
        format!(
            "https://huggingface.co/{repo}/resolve/main/{}",
            self.file_name()
        )
    }

    pub fn path_in(self, models_dir: &Path) -> PathBuf {
        models_dir.join(self.file_name())
    }

    /// Approximate download size in MB, for UI.
    pub fn approx_mb(self) -> u32 {
        match self {
            LocalModel::Qwen3_4B => 2500,
            LocalModel::Qwen3_1_7B => 1100,
        }
    }

    /// Approximate memory while loaded, in MB, for UI (measured on a Mac;
    /// CPU-only runs use more).
    pub fn approx_ram_mb(self) -> u32 {
        match self {
            LocalModel::Qwen3_4B => 3000,
            LocalModel::Qwen3_1_7B => 1700,
        }
    }

    /// A short description, for UI.
    pub fn label(self) -> &'static str {
        match self {
            LocalModel::Qwen3_4B => "Qwen3 4B · best quality",
            LocalModel::Qwen3_1_7B => "Qwen3 1.7B · smaller, faster",
        }
    }

    pub fn is_downloaded(self, models_dir: &Path) -> bool {
        self.path_in(models_dir).is_file()
    }

    pub async fn download(
        self,
        models_dir: &Path,
        progress: impl FnMut(u64, Option<u64>),
    ) -> Result<PathBuf, ProviderError> {
        download_file(&self.url(), &self.path_in(models_dir), progress).await
    }
}

/// Downloads `url` to `dest` (via a `.part` file, so an interrupted
/// download never looks complete), unless it's already there.
async fn download_file(
    url: &str,
    dest: &Path,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<PathBuf, ProviderError> {
    if dest.is_file() {
        return Ok(dest.to_path_buf());
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(io_error)?;
    }
    let mut part = dest.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);

    let mut resp = reqwest::get(url)
        .await
        .map_err(|e| ProviderError::Network(e.to_string()))?
        .error_for_status()
        .map_err(|e| ProviderError::Network(e.to_string()))?;
    let total = resp.content_length();

    let mut file = BufWriter::with_capacity(1 << 20, File::create(&part).map_err(io_error)?);
    let mut done = 0u64;
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| ProviderError::Network(e.to_string()))?
    {
        file.write_all(&chunk).map_err(io_error)?;
        done += chunk.len() as u64;
        progress(done, total);
    }
    file.flush().map_err(io_error)?;
    drop(file);

    if total.is_some_and(|t| t != done) {
        let _ = std::fs::remove_file(&part);
        return Err(ProviderError::Network("download was incomplete".into()));
    }
    std::fs::rename(&part, dest).map_err(io_error)?;
    Ok(dest.to_path_buf())
}

/// Downloads a catalog model (Whisper or local text) by id.
pub async fn download_by_id(
    id: &str,
    models_dir: &Path,
    progress: impl FnMut(u64, Option<u64>),
) -> Result<PathBuf, ProviderError> {
    if let Some(m) = WhisperModel::from_id(id) {
        m.download(models_dir, progress).await
    } else if let Some(m) = LocalModel::from_id(id) {
        m.download(models_dir, progress).await
    } else {
        Err(ProviderError::Unavailable(format!(
            "unknown model \"{id}\""
        )))
    }
}

fn io_error(e: std::io::Error) -> ProviderError {
    ProviderError::Unavailable(format!("couldn't write model file: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        for m in WhisperModel::ALL {
            assert_eq!(WhisperModel::from_id(m.id()), Some(m));
        }
        for m in LocalModel::ALL {
            assert_eq!(LocalModel::from_id(m.id()), Some(m));
        }
        assert_eq!(
            WhisperModel::BaseEn.url(),
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin"
        );
    }
}
