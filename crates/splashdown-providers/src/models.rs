//! Downloadable local models (spec §5.2, §5.6): speech models run by
//! transcribe.cpp, and the built-in text models run by llama.cpp. Both are
//! GGUF files fetched from Hugging Face when the user asks, never bundled.
//! Downloading doesn't need the `local-speech` feature, so the app can fetch
//! a model before loading it.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use splashdown_core::ProviderError;

/// Local speech-to-text models, run by transcribe.cpp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechModel {
    /// NVIDIA Nemotron streaming, English: transcribes while you speak, so
    /// the text is ready moments after you stop. The default.
    NemotronEn,
    /// NVIDIA Nemotron 3.5 streaming: the same, in 32 languages.
    Nemotron,
    /// NVIDIA Parakeet TDT v2, English, transcribed when you stop.
    ParakeetV2,
    /// NVIDIA Parakeet TDT v3, 25 European languages.
    ParakeetV3,
    /// OpenAI Whisper base.en: small and English-only.
    WhisperBaseEn,
    /// OpenAI Whisper small.en.
    WhisperSmallEn,
    /// OpenAI Whisper large-v3-turbo, 4-bit: about 100 languages.
    WhisperTurbo,
}

impl SpeechModel {
    pub const ALL: [SpeechModel; 7] = [
        SpeechModel::NemotronEn,
        SpeechModel::Nemotron,
        SpeechModel::ParakeetV2,
        SpeechModel::ParakeetV3,
        SpeechModel::WhisperBaseEn,
        SpeechModel::WhisperSmallEn,
        SpeechModel::WhisperTurbo,
    ];

    pub fn id(self) -> &'static str {
        match self {
            SpeechModel::NemotronEn => "nemotron-en",
            SpeechModel::Nemotron => "nemotron",
            SpeechModel::ParakeetV2 => "parakeet-v2",
            SpeechModel::ParakeetV3 => "parakeet-v3",
            SpeechModel::WhisperBaseEn => "base.en",
            SpeechModel::WhisperSmallEn => "small.en",
            SpeechModel::WhisperTurbo => "large-v3-turbo",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        // The whisper.cpp-era id of the turbo model.
        let id = if id == "large-v3-turbo-q5_0" {
            "large-v3-turbo"
        } else {
            id
        };
        Self::ALL.into_iter().find(|m| m.id() == id)
    }

    /// The model's Hugging Face repo and file.
    fn source(self) -> (&'static str, &'static str) {
        match self {
            SpeechModel::NemotronEn => (
                "nemotron-speech-streaming-en-0.6b",
                "nemotron-speech-streaming-en-0.6b-Q4_K_M.gguf",
            ),
            SpeechModel::Nemotron => (
                "nemotron-3.5-asr-streaming-0.6b",
                "nemotron-3.5-asr-streaming-0.6b-Q4_K_M.gguf",
            ),
            SpeechModel::ParakeetV2 => ("parakeet-tdt-0.6b-v2", "parakeet-tdt-0.6b-v2-Q4_K_M.gguf"),
            SpeechModel::ParakeetV3 => ("parakeet-tdt-0.6b-v3", "parakeet-tdt-0.6b-v3-Q4_K_M.gguf"),
            SpeechModel::WhisperBaseEn => ("whisper-base.en", "whisper-base.en-Q8_0.gguf"),
            SpeechModel::WhisperSmallEn => ("whisper-small.en", "whisper-small.en-Q8_0.gguf"),
            SpeechModel::WhisperTurbo => (
                "whisper-large-v3-turbo",
                "whisper-large-v3-turbo-Q5_K_M.gguf",
            ),
        }
    }

    pub fn file_name(self) -> &'static str {
        self.source().1
    }

    pub fn url(self) -> String {
        let (repo, file) = self.source();
        format!("https://huggingface.co/handy-computer/{repo}-gguf/resolve/main/{file}")
    }

    /// Approximate download size in MB, for UI.
    pub fn approx_mb(self) -> u32 {
        match self {
            SpeechModel::NemotronEn | SpeechModel::ParakeetV2 => 475,
            SpeechModel::Nemotron => 495,
            SpeechModel::ParakeetV3 => 485,
            SpeechModel::WhisperBaseEn => 84,
            SpeechModel::WhisperSmallEn => 269,
            SpeechModel::WhisperTurbo => 619,
        }
    }

    /// Only understands English.
    pub fn english_only(self) -> bool {
        matches!(
            self,
            SpeechModel::NemotronEn
                | SpeechModel::ParakeetV2
                | SpeechModel::WhisperBaseEn
                | SpeechModel::WhisperSmallEn
        )
    }

    /// Transcribes while the user speaks.
    pub fn streams(self) -> bool {
        matches!(self, SpeechModel::NemotronEn | SpeechModel::Nemotron)
    }

    /// What loading needs to know before the file is read: streaming
    /// models stay on the CPU, where each small piece is quickest.
    #[cfg(feature = "local-speech")]
    pub fn profile(self) -> crate::SpeechProfile {
        crate::SpeechProfile {
            cpu: self.streams(),
            english_only: self.english_only(),
            streams: self.streams(),
        }
    }

    /// A short description, for UI.
    pub fn label(self) -> &'static str {
        match self {
            SpeechModel::NemotronEn => "Nemotron · English · fastest, text as you speak",
            SpeechModel::Nemotron => "Nemotron 3.5 · 32 languages · text as you speak",
            SpeechModel::ParakeetV2 => "Parakeet v2 · English",
            SpeechModel::ParakeetV3 => "Parakeet v3 · 25 European languages",
            SpeechModel::WhisperBaseEn => "Whisper base · English · smallest",
            SpeechModel::WhisperSmallEn => "Whisper small · English",
            SpeechModel::WhisperTurbo => "Whisper large-v3-turbo · about 100 languages",
        }
    }

    /// The model's license, which the app shows next to it.
    pub fn license(self) -> &'static str {
        match self {
            SpeechModel::NemotronEn => "NVIDIA Open Model License",
            SpeechModel::Nemotron => "OpenMDW-1.1",
            SpeechModel::ParakeetV2 | SpeechModel::ParakeetV3 => "CC-BY-4.0 (NVIDIA)",
            SpeechModel::WhisperBaseEn
            | SpeechModel::WhisperSmallEn
            | SpeechModel::WhisperTurbo => "MIT (OpenAI)",
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

/// Downloads a catalog model (speech or local text) by id.
pub async fn download_by_id(
    id: &str,
    models_dir: &Path,
    progress: impl FnMut(u64, Option<u64>),
) -> Result<PathBuf, ProviderError> {
    if let Some(m) = SpeechModel::from_id(id) {
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
        for m in SpeechModel::ALL {
            assert_eq!(SpeechModel::from_id(m.id()), Some(m));
        }
        for m in LocalModel::ALL {
            assert_eq!(LocalModel::from_id(m.id()), Some(m));
        }
        assert_eq!(
            SpeechModel::from_id("large-v3-turbo-q5_0"),
            Some(SpeechModel::WhisperTurbo)
        );
        assert_eq!(
            SpeechModel::NemotronEn.url(),
            "https://huggingface.co/handy-computer/nemotron-speech-streaming-en-0.6b-gguf/resolve/main/nemotron-speech-streaming-en-0.6b-Q4_K_M.gguf"
        );
    }
}
