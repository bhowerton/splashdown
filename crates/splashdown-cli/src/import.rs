//! `splashdown-cli words import`: adding another dictation app's words,
//! corrections and snippets to the settings.

use super::*;

/// Merges another app's words into the config file and tells the running
/// app to read it again.
pub(super) fn import_words(
    file: &std::path::Path,
    dry_run: bool,
    config: Option<PathBuf>,
) -> Result<()> {
    use splashdown_config::import;

    let imported = import::read(file).with_context(|| format!("importing {}", file.display()))?;
    let own_config = config.is_none();
    let path = match config {
        Some(p) => p,
        None => manage::paths()?.config_file(),
    };
    let mut settings = splashdown_config::Settings::load(&path)?;
    let merged = import::merge(&mut settings, &imported);
    let added = merged.vocabulary + merged.replacements + merged.snippets;
    println!(
        "{}: {} vocabulary word(s), {} correction(s), {} snippet(s) to add",
        imported.kind, merged.vocabulary, merged.replacements, merged.snippets
    );
    if merged.duplicates > 0 {
        println!("{} already there", merged.duplicates);
    }
    if merged.conflicts > 0 {
        println!(
            "{} kept as you have them (the file says something different)",
            merged.conflicts
        );
    }
    if imported.skipped > 0 {
        println!("{} line(s) or entries couldn't be used", imported.skipped);
    }
    if dry_run || added == 0 {
        if dry_run {
            println!("Nothing saved (--dry-run).");
        }
        return Ok(());
    }
    settings.save(&path).map_err(anyhow::Error::msg)?;
    if !own_config {
        println!("Saved to {}.", path.display());
        return Ok(());
    }
    match splashdown_control::send(&Request::ReloadSettings) {
        Ok(_) => println!("Saved; Splashdown is using them now."),
        Err(_) => println!("Saved; Splashdown will use them when it starts."),
    }
    Ok(())
}
