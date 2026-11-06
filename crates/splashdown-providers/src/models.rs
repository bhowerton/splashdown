//! Downloadable local models: speech models run by
//! transcribe.cpp, and the built-in text models run by llama.cpp. Both are
//! GGUF files fetched from Hugging Face when the user asks, never bundled.
//! Downloading doesn't need the `local-speech` feature, so the app can fetch
//! a model before loading it.

use std::path::{Path, PathBuf};

use splashdown_core::ProviderError;

use crate::download::*;
pub use crate::download::{in_use, rename_patiently};

/// Local speech-to-text models, run by transcribe.cpp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechModel {
    /// NVIDIA Nemotron streaming, English: transcribes while you speak, so
    /// the text is ready moments after you stop.
    NemotronEn,
    /// NVIDIA Nemotron 3.5 streaming: the same, in 32 languages. The default.
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
    /// In order of preference, so the default comes first.
    pub const ALL: [SpeechModel; 7] = [
        SpeechModel::Nemotron,
        SpeechModel::NemotronEn,
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
            SpeechModel::NemotronEn => "Nemotron · English · text as you speak",
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
    /// The same Qwen3 4B, packed smaller (IQ3_XXS). ~1.7 GB, and about
    /// 2.6 GB of memory on the CPU: the most the eval gets from a computer
    /// with about 8 GB (74%, against 52–55% for Qwen3 1.7B, which uses as
    /// much memory there).
    Qwen3_4BCompact,
    /// Qwen3 1.7B, 4-bit, thinking off (by `splashdown-llm`). Apache-2.0.
    /// ~1.1 GB. Kept for those who chose it; the compact 4B does better.
    Qwen3_1_7B,
}

impl LocalModel {
    pub const ALL: [LocalModel; 3] = [
        LocalModel::Qwen3_4B,
        LocalModel::Qwen3_4BCompact,
        LocalModel::Qwen3_1_7B,
    ];

    pub fn id(self) -> &'static str {
        match self {
            LocalModel::Qwen3_4B => "qwen3-4b",
            LocalModel::Qwen3_4BCompact => "qwen3-4b-compact",
            LocalModel::Qwen3_1_7B => "qwen3-1.7b",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id() == id)
    }

    pub fn file_name(self) -> &'static str {
        match self {
            LocalModel::Qwen3_4B => "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
            LocalModel::Qwen3_4BCompact => "Qwen3-4B-Instruct-2507-UD-IQ3_XXS.gguf",
            LocalModel::Qwen3_1_7B => "Qwen3-1.7B-Q4_K_M.gguf",
        }
    }

    pub fn url(self) -> String {
        let repo = match self {
            LocalModel::Qwen3_4B | LocalModel::Qwen3_4BCompact => {
                "unsloth/Qwen3-4B-Instruct-2507-GGUF"
            }
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
            LocalModel::Qwen3_4BCompact => 1670,
            LocalModel::Qwen3_1_7B => 1100,
        }
    }

    /// Approximate memory while loaded, in MB, for UI (measured on a Mac;
    /// CPU-only runs use more).
    pub fn approx_ram_mb(self) -> u32 {
        match self {
            LocalModel::Qwen3_4B => 3000,
            LocalModel::Qwen3_4BCompact => 2300,
            LocalModel::Qwen3_1_7B => 1700,
        }
    }

    /// A short description, for UI.
    pub fn label(self) -> &'static str {
        match self {
            LocalModel::Qwen3_4B => "Qwen3 4B · best quality",
            LocalModel::Qwen3_4BCompact => "Qwen3 4B compact · for about 8 GB of memory",
            LocalModel::Qwen3_1_7B => "Qwen3 1.7B · smallest",
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

/// Downloads a catalog model (speech or local text) by id.
/// NVIDIA's streaming Sortformer: who spoke when, for up to four speakers.
/// Tells the other people on a meeting apart. NVIDIA Open Model License,
/// 8-bit, about 139 MB.
pub struct Diarizer;

impl Diarizer {
    pub const ID: &'static str = "sortformer";
    pub const FILE: &'static str = "diar_streaming_sortformer_4spk-v2.1-Q8_0.gguf";
    /// Approximate download size in MB, for UI.
    pub const APPROX_MB: u32 = 139;

    pub fn url() -> String {
        format!(
            "https://huggingface.co/handy-computer/diar_streaming_sortformer_4spk-v2.1-gguf/resolve/main/{}",
            Self::FILE
        )
    }

    pub fn path_in(models_dir: &Path) -> PathBuf {
        models_dir.join(Self::FILE)
    }

    pub fn is_downloaded(models_dir: &Path) -> bool {
        Self::path_in(models_dir).is_file()
    }
}

/// Voiceprints of meeting speakers, to know someone again in a later
/// meeting: NVIDIA TitaNet-small (NeMo Toolkit license, Apache-2.0), as
/// sherpa-onnx exports it, about 38 MB.
pub struct VoiceModel;

impl VoiceModel {
    pub const ID: &'static str = "titanet-small";
    pub const FILE: &'static str = "nemo_en_titanet_small.onnx";
    /// Approximate download size in MB, for UI.
    pub const APPROX_MB: u32 = 38;

    pub fn url() -> String {
        format!(
            "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/{}",
            Self::FILE
        )
    }

    pub fn path_in(models_dir: &Path) -> PathBuf {
        models_dir.join(Self::FILE)
    }

    pub fn is_downloaded(models_dir: &Path) -> bool {
        Self::path_in(models_dir).is_file()
    }
}

/// Finds notes by what they mean rather than the words they use, for Ask.
/// Qwen3 Embedding 0.6B, 8-bit, run by `splashdown-llm`; over 100
/// languages. Apache-2.0, about 639 MB.
pub struct Embedder;

impl Embedder {
    pub const ID: &'static str = "qwen3-embedding";
    pub const FILE: &'static str = "Qwen3-Embedding-0.6B-Q8_0.gguf";
    /// Approximate download size in MB, for UI.
    pub const APPROX_MB: u32 = 639;

    pub fn url() -> String {
        format!(
            "https://huggingface.co/Qwen/Qwen3-Embedding-0.6B-GGUF/resolve/main/{}",
            Self::FILE
        )
    }

    pub fn path_in(models_dir: &Path) -> PathBuf {
        models_dir.join(Self::FILE)
    }

    pub fn is_downloaded(models_dir: &Path) -> bool {
        Self::path_in(models_dir).is_file()
    }
}

/// The natural reading voice: Gradium's Phonon 31M, English, run in
/// process by `ptts` (feature `natural-voice`). Apache-2.0, about 68 MB:
/// the weights (8-bit), the tokenizer, and the British and American voices,
/// each a small embedding. Pinned to a revision of each repo, so what's
/// downloaded doesn't change under us.
pub struct NaturalVoice;

/// One of the natural voice's voices, from its catalog.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NaturalVoiceEntry {
    pub name: String,
    /// The file, relative to the catalog.
    pub filename: String,
    pub language: String,
    /// "en-us" or "en-gb".
    pub locale: String,
    #[serde(default)]
    pub gender: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

impl NaturalVoice {
    pub const ID: &'static str = "phonon-en";
    pub const DIR: &'static str = "phonon-31m-en";
    /// Approximate download size in MB, for UI.
    pub const APPROX_MB: u32 = 68;
    const MODEL_REPO: &'static str = "gradium/phonon-31m-en";
    const MODEL_REVISION: &'static str = "92741c5b07d085ff840e26fea8ce43110b87852f";
    const VOICES_REPO: &'static str = "gradium/phonon-voices";
    const VOICES_REVISION: &'static str = "0ee17205f254b5abc43b266c8a0f215565c352f7";
    /// The model's files and their sizes, the weights last but one (the
    /// voice catalog, written last, marks the download complete).
    const FILES: [(&'static str, u64); 4] = [
        ("config.json", 4_634),
        ("tokenizer.json", 277_967),
        ("default-voice.safetensors", 256_136),
        ("model.q8.gguf", 64_512_352),
    ];
    const VOICE_BYTES: u64 = 256_136;
    /// The catalog of the voices we keep.
    pub const CATALOG: &'static str = "voices.jsonl";

    pub fn dir_in(models_dir: &Path) -> PathBuf {
        models_dir.join(Self::DIR)
    }

    pub fn is_downloaded(models_dir: &Path) -> bool {
        let dir = Self::dir_in(models_dir);
        dir.join(Self::CATALOG).is_file() && Self::FILES.iter().all(|(f, _)| dir.join(f).is_file())
    }

    /// The voices that were downloaded, British and American.
    pub fn voices(models_dir: &Path) -> Vec<NaturalVoiceEntry> {
        let path = Self::dir_in(models_dir).join(Self::CATALOG);
        std::fs::read_to_string(path)
            .map(|text| Self::readable_voices(&text))
            .unwrap_or_default()
    }

    /// The catalog's British and American voices (not the character
    /// voices, which aren't for reading text back).
    fn readable_voices(catalog: &str) -> Vec<NaturalVoiceEntry> {
        catalog
            .lines()
            .filter_map(|line| serde_json::from_str::<NaturalVoiceEntry>(line).ok())
            .filter(|v| {
                v.language == "en"
                    && matches!(v.locale.as_str(), "en-us" | "en-gb")
                    && !v.filename.contains("..")
            })
            .collect()
    }

    fn url(repo: &str, revision: &str, file: &str) -> String {
        format!("https://huggingface.co/{repo}/resolve/{revision}/{file}")
    }

    /// Downloads the model and its voices into `models_dir`. `progress`
    /// receives `(downloaded_bytes, total_bytes)` across all the files.
    pub async fn download(
        models_dir: &Path,
        mut progress: impl FnMut(u64, Option<u64>),
    ) -> Result<PathBuf, ProviderError> {
        let dir = Self::dir_in(models_dir);
        std::fs::create_dir_all(dir.join("voices")).map_err(io_error)?;
        let all = dir.join("voices.all.jsonl");
        download_file(
            &Self::url(Self::VOICES_REPO, Self::VOICES_REVISION, "voices.jsonl"),
            &all,
            |_, _| {},
        )
        .await?;
        let catalog = std::fs::read_to_string(&all).map_err(io_error)?;
        let voices = Self::readable_voices(&catalog);
        if voices.is_empty() {
            return Err(ProviderError::Unavailable(
                "the voice list had no English voices".into(),
            ));
        }
        let total = Self::FILES.iter().map(|(_, n)| n).sum::<u64>()
            + Self::VOICE_BYTES * voices.len() as u64;
        let mut done = 0;
        let files = Self::FILES
            .iter()
            .map(|(f, n)| (Self::url(Self::MODEL_REPO, Self::MODEL_REVISION, f), *f, *n));
        let voice_files = voices.iter().map(|v| {
            (
                Self::url(Self::VOICES_REPO, Self::VOICES_REVISION, &v.filename),
                v.filename.as_str(),
                Self::VOICE_BYTES,
            )
        });
        for (url, file, size) in voice_files.chain(files) {
            download_file(&url, &dir.join(file), |got, _| {
                progress(done + got.min(size), Some(total))
            })
            .await?;
            done += size;
        }
        let kept = voices
            .iter()
            .filter_map(|v| serde_json::to_string(v).ok())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(dir.join(Self::CATALOG), kept + "\n").map_err(io_error)?;
        let _ = std::fs::remove_file(&all);
        progress(total, Some(total));
        Ok(dir)
    }
}

pub async fn download_by_id(
    id: &str,
    models_dir: &Path,
    progress: impl FnMut(u64, Option<u64>),
) -> Result<PathBuf, ProviderError> {
    if id == NaturalVoice::ID {
        NaturalVoice::download(models_dir, progress).await
    } else if id == Diarizer::ID {
        download_file(&Diarizer::url(), &Diarizer::path_in(models_dir), progress).await
    } else if id == Embedder::ID {
        download_file(&Embedder::url(), &Embedder::path_in(models_dir), progress).await
    } else if id == VoiceModel::ID {
        download_file(
            &VoiceModel::url(),
            &VoiceModel::path_in(models_dir),
            progress,
        )
        .await
    } else if let Some(m) = SpeechModel::from_id(id) {
        m.download(models_dir, progress).await
    } else if let Some(m) = LocalModel::from_id(id) {
        m.download(models_dir, progress).await
    } else {
        Err(ProviderError::Unavailable(format!(
            "unknown model \"{id}\""
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_british_and_american_voices() {
        let catalog = concat!(
            r#"{"filename": "voices/a.safetensors", "name": "Zoey", "language": "en", "locale": "en-us", "gender": "female", "description": "Upbeat."}"#,
            "\n",
            r#"{"filename": "voices/b.safetensors", "name": "Pirate", "language": "en", "locale": "characters"}"#,
            "\n",
            r#"{"filename": "voices/c.safetensors", "name": "Léa", "language": "fr", "locale": "fr-fr"}"#,
            "\n",
            r#"{"filename": "../x.safetensors", "name": "Sneaky", "language": "en", "locale": "en-gb"}"#,
        );
        let voices = NaturalVoice::readable_voices(catalog);
        assert_eq!(voices.len(), 1);
        assert_eq!(voices[0].name, "Zoey");
        assert_eq!(voices[0].gender.as_deref(), Some("female"));
    }

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
