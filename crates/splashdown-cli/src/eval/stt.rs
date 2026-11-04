//! `eval stt` and `make-audio`: speech-to-text scored by word error rate
//! (eval/stt.toml, eval/stt-languages.toml).

use super::*;

#[derive(Deserialize)]
pub(super) struct SttFile {
    pub(super) case: Vec<SttCase>,
}

#[derive(Deserialize)]
pub(super) struct SttCase {
    pub(super) id: String,
    pub(super) text: String,
    /// The language spoken (ISO 639-1). Unset means English.
    pub(super) language: Option<String>,
    /// The `say` voice for `make-audio`, for a case in another language.
    pub(super) voice: Option<String>,
}

pub(super) async fn stt(
    path: &Path,
    audio_dir: &Path,
    verbose: bool,
    detect_language: bool,
    providers: &ProviderOpts,
) -> Result<()> {
    let file: SttFile = toml::from_str(
        &std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?,
    )?;
    let settings = opts::settings(providers)?;
    let stt = opts::build_stt(&settings, &Secrets::from_env())?;
    let name = stt.name();
    eprintln!("Speech-to-text eval · {name} · {} cases\n", file.case.len());

    let mut latencies = Vec::new();
    let mut total_wer = 0.0;
    let mut errors = 0;
    // Error rate per language, for sets with more than one.
    let mut by_language: std::collections::BTreeMap<String, (f64, usize)> = Default::default();
    for case in &file.case {
        let language = case.language.clone().unwrap_or_else(|| "en".into());
        let hints = splashdown_core::SttHints {
            vocabulary: settings.general.vocabulary.clone(),
            language: (!detect_language).then(|| language.clone()),
            ..Default::default()
        };
        let wav = audio_dir.join(format!("{}.wav", case.id));
        let clip = splashdown_audio::wav::read(&wav).with_context(|| {
            format!(
                "reading {} (run `splashdown-cli eval make-audio`)",
                wav.display()
            )
        })?;
        let Some(clip) = splashdown_audio::prepare(&clip) else {
            bail!("no speech found in {}", wav.display());
        };
        let started = Instant::now();
        let result = stt.transcribe(&clip, &hints).await;
        let elapsed = started.elapsed();
        latencies.push(elapsed);
        match result {
            Ok(t) => {
                let rate = wer(&case.text, &t.text);
                total_wer += rate;
                let entry = by_language.entry(language.clone()).or_default();
                entry.0 += rate;
                entry.1 += 1;
                println!(
                    "{:<26} {:>5.2}s  WER {:>5.1}%",
                    case.id,
                    elapsed.as_secs_f32(),
                    rate * 100.0
                );
                if verbose || rate > 0.0 {
                    println!("      │ heard: {}", t.text);
                }
            }
            Err(e) => {
                errors += 1;
                total_wer += 1.0;
                let entry = by_language.entry(language.clone()).or_default();
                entry.0 += 1.0;
                entry.1 += 1;
                println!(
                    "{:<26} {:>5.2}s  error: {e}",
                    case.id,
                    elapsed.as_secs_f32()
                );
            }
        }
    }
    let n = file.case.len().max(1) as f64;
    if by_language.len() > 1 {
        let rates: Vec<String> = by_language
            .iter()
            .map(|(lang, (sum, count))| format!("{lang} {:.0}%", sum / *count as f64 * 100.0))
            .collect();
        println!("\nby language: {}", rates.join(" · "));
    }
    latencies.sort();
    println!(
        "\n{name}: mean WER {:.1}% · {errors} errors · latency p50 {:.2}s p90 {:.2}s",
        total_wer / n * 100.0,
        percentile(&latencies, 0.5),
        percentile(&latencies, 0.9)
    );
    Ok(())
}

pub(super) fn make_audio(path: &Path, audio_dir: &Path, voice: &str) -> Result<()> {
    if !cfg!(target_os = "macos") {
        bail!("make-audio uses macOS `say`; copy eval/audio from a Mac instead");
    }
    let file: SttFile = toml::from_str(&std::fs::read_to_string(path)?)?;
    std::fs::create_dir_all(audio_dir)?;
    for case in &file.case {
        let out = audio_dir.join(format!("{}.wav", case.id));
        let status = std::process::Command::new("say")
            .args([
                "-v",
                case.voice.as_deref().unwrap_or(voice),
                "--file-format=WAVE",
                "--data-format=LEI16@16000",
                "-o",
            ])
            .arg(&out)
            .arg(&case.text)
            .status()?;
        if !status.success() {
            bail!("`say` failed for {}", case.id);
        }
        println!("{}", out.display());
    }
    Ok(())
}

/// Lowercased words with punctuation stripped (apostrophes kept).
pub(super) fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for word in text.split(|c: char| !(c.is_alphanumeric() || c == '\'' || is_unspaced(c))) {
        let mut rest = String::new();
        for c in word.chars() {
            if is_unspaced(c) {
                if !rest.is_empty() {
                    out.push(std::mem::take(&mut rest).to_lowercase());
                }
                out.push(c.to_string());
            } else {
                rest.push(c);
            }
        }
        if !rest.is_empty() {
            out.push(rest.to_lowercase());
        }
    }
    out
}

/// Chinese and Japanese aren't written with spaces between words: each of
/// their characters counts as a word (so the rate is a character error
/// rate there, the usual measure).
pub(super) fn is_unspaced(c: char) -> bool {
    matches!(c, '\u{3040}'..='\u{30ff}' | '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}')
}

/// Word error rate: word-level edit distance / reference length.
pub(super) fn wer(reference: &str, hypothesis: &str) -> f64 {
    let r = words(reference);
    let h = words(hypothesis);
    if r.is_empty() {
        return if h.is_empty() { 0.0 } else { 1.0 };
    }
    let mut prev: Vec<usize> = (0..=h.len()).collect();
    for (i, rw) in r.iter().enumerate() {
        let mut cur = vec![i + 1; h.len() + 1];
        for (j, hw) in h.iter().enumerate() {
            let sub = prev[j] + usize::from(rw != hw);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[h.len()] as f64 / r.len() as f64
}
