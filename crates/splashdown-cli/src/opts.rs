//! Shared command-line options and provider setup.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Args;
use splashdown_config::{AiProvider, Paths, Secrets, Settings, SpeechProvider};
use splashdown_core::{Pipeline, PromptContext, SpeechToText, SttHints, TextModel};

#[derive(Args, Clone, Default)]
pub struct ProviderOpts {
    /// Speech-to-text: auto, openai, openrouter, whisper-local, apple.
    #[arg(long)]
    pub stt: Option<SpeechProvider>,
    #[arg(long)]
    pub stt_model: Option<String>,
    /// AI pass: auto, openai, openrouter, custom, apple, off.
    #[arg(long)]
    pub ai: Option<AiProvider>,
    #[arg(long)]
    pub ai_model: Option<String>,
    /// With a local AI pass: send instructions and transforms to a cloud
    /// model (OpenAI or OpenRouter key).
    #[arg(long)]
    pub cloud_instructions: bool,
    /// Base URL for `--ai custom` (default: Ollama on localhost).
    #[arg(long)]
    pub base_url: Option<String>,
    /// Config file (default: the app's config.toml).
    #[arg(long)]
    pub config: Option<PathBuf>,
}

#[derive(Args, Clone, Default)]
pub struct ContextOpts {
    /// Preset formatting instructions, e.g. "Casual tone, no sign-off."
    #[arg(long)]
    pub preset: Option<String>,
    /// App the text is going into, e.g. "Slack".
    #[arg(long)]
    pub app: Option<String>,
    /// Extra vocabulary, comma-separated.
    #[arg(long, value_delimiter = ',')]
    pub vocab: Vec<String>,
    /// Spoken language (ISO-639-1), e.g. "en".
    #[arg(long)]
    pub language: Option<String>,
    /// Languages mixed mid-sentence, comma-separated (with auto-detect).
    #[arg(long, value_delimiter = ',')]
    pub languages: Vec<String>,
    /// How much plain dictation may be reworded: light, medium or high.
    #[arg(long, value_parser = parse_cleanup)]
    pub cleanup: Option<splashdown_core::CleanupLevel>,
}

fn parse_cleanup(s: &str) -> Result<splashdown_core::CleanupLevel, String> {
    use splashdown_core::CleanupLevel;
    match s {
        "light" => Ok(CleanupLevel::Light),
        "medium" => Ok(CleanupLevel::Medium),
        "high" => Ok(CleanupLevel::High),
        _ => Err(format!(
            "unknown cleanup level \"{s}\" (light, medium, high)"
        )),
    }
}

impl ContextOpts {
    pub fn prompt_context(&self, settings: &Settings) -> PromptContext {
        PromptContext {
            preset_instructions: self.preset.clone(),
            app_name: self.app.clone(),
            user_name: settings.general.user_name.clone(),
            vocabulary: self.vocabulary(settings),
            cleanup: self.cleanup.unwrap_or_default(),
            ..PromptContext::default()
        }
    }

    pub fn hints(&self, settings: &Settings) -> SttHints {
        SttHints {
            vocabulary: self.vocabulary(settings),
            language: self
                .language
                .clone()
                .or_else(|| settings.general.language.clone()),
            languages: if self.languages.is_empty() {
                settings.general.languages.clone()
            } else {
                self.languages.clone()
            },
        }
    }

    fn vocabulary(&self, settings: &Settings) -> Vec<String> {
        let mut v = settings.general.vocabulary.clone();
        v.extend(self.vocab.iter().cloned());
        v
    }
}

pub fn paths() -> Result<Paths> {
    Paths::resolve().context("couldn't find the home directory")
}

/// Loads settings and applies command-line overrides.
pub fn settings(opts: &ProviderOpts) -> Result<Settings> {
    let path = match &opts.config {
        Some(p) => p.clone(),
        None => paths()?.config_file(),
    };
    let mut s = Settings::load(&path)?;
    if let Some(p) = opts.stt {
        s.speech.provider = p;
        s.speech.model = None;
    }
    if opts.stt_model.is_some() {
        s.speech.model = opts.stt_model.clone();
    }
    if let Some(p) = opts.ai {
        s.ai.provider = p;
        s.ai.model = None;
    }
    if opts.ai_model.is_some() {
        s.ai.model = opts.ai_model.clone();
        // An explicitly chosen model is measured alone, without fallbacks.
        s.ai.fallback_models = Some(Vec::new());
    }
    if opts.cloud_instructions {
        s.ai.cloud_for_instructions = true;
    }
    if opts.base_url.is_some() {
        s.ai.base_url = opts.base_url.clone();
    }
    Ok(s)
}

pub struct Setup {
    pub settings: Settings,
    pub pipeline: Pipeline,
}

impl Setup {
    pub fn new(opts: &ProviderOpts) -> Result<Self> {
        let settings = settings(opts)?;
        let secrets = Secrets::from_env();
        let stt = build_stt(&settings, &secrets)?;
        let llm = build_llm(&settings, &secrets)?;
        Ok(Self {
            pipeline: Pipeline::new(stt, llm)
                .with_instruction_model(build_instruction_llm(&settings, &secrets)?),
            settings,
        })
    }

    /// For text-only commands: no speech-to-text provider is loaded.
    pub fn new_text_only(opts: &ProviderOpts) -> Result<Self> {
        let settings = settings(opts)?;
        let secrets = Secrets::from_env();
        let llm = build_llm(&settings, &secrets)?;
        Ok(Self {
            pipeline: Pipeline::new(Arc::new(NoStt), llm)
                .with_instruction_model(build_instruction_llm(&settings, &secrets)?),
            settings,
        })
    }
}

fn on_device(settings: &Settings) -> splashdown_config::Situation {
    let models_dir = paths().map(|p| p.models_dir).unwrap_or_default();
    splashdown_providers::factory::on_device(settings.general.language.as_deref(), &models_dir)
}

pub fn build_stt(settings: &Settings, secrets: &Secrets) -> Result<Arc<dyn SpeechToText>> {
    let choice = settings.resolve_speech_with(secrets, on_device(settings))?;
    Ok(splashdown_providers::factory::speech(
        &choice,
        secrets,
        &paths()?.models_dir,
    )?)
}

pub fn build_llm(settings: &Settings, secrets: &Secrets) -> Result<Option<Arc<dyn TextModel>>> {
    settings
        .resolve_ai_with(secrets, on_device(settings))?
        .map(|choice| splashdown_providers::factory::text_model(&choice, secrets))
        .transpose()
        .map_err(Into::into)
}

pub fn build_instruction_llm(
    settings: &Settings,
    secrets: &Secrets,
) -> Result<Option<Arc<dyn TextModel>>> {
    settings
        .resolve_instruction_ai_with(secrets, on_device(settings))
        .map(|choice| splashdown_providers::factory::text_model(&choice, secrets))
        .transpose()
        .map_err(Into::into)
}

pub fn print_config(opts: &ProviderOpts) -> Result<()> {
    let paths = paths()?;
    let settings = settings(opts)?;
    let secrets = Secrets::from_env();
    println!("config file : {}", paths.config_file().display());
    println!("models      : {}", paths.models_dir.display());
    println!("keys        : {secrets:?}");
    match settings.resolve_speech_with(&secrets, on_device(&settings)) {
        Ok(c) => println!("speech      : {:?} / {}", c.provider, c.model),
        Err(e) => println!("speech      : error: {e}"),
    }
    match settings.resolve_ai_with(&secrets, on_device(&settings)) {
        Ok(Some(c)) => println!(
            "ai          : {:?} / {}{}",
            c.provider,
            c.model,
            c.base_url.map(|u| format!(" @ {u}")).unwrap_or_default()
        ),
        Ok(None) => println!("ai          : off"),
        Err(e) => println!("ai          : error: {e}"),
    }
    Ok(())
}

/// Placeholder speech-to-text for text-only commands.
struct NoStt;

#[async_trait::async_trait]
impl SpeechToText for NoStt {
    fn name(&self) -> String {
        "none".into()
    }
    async fn transcribe(
        &self,
        _: &splashdown_core::AudioClip,
        _: &SttHints,
    ) -> splashdown_core::ProviderResult<splashdown_core::Transcript> {
        Err(splashdown_core::ProviderError::Unavailable(
            "no speech-to-text in text-only mode".into(),
        ))
    }
}
