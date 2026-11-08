//! Language codes and names, shared by everything that deals in them:
//! the spoken language, translation, reading aloud, meetings.

/// The language of a tag, lowercase: "de-DE" → "de", "pt_BR" → "pt".
pub fn base(tag: &str) -> String {
    tag.split(['-', '_'])
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

/// A language's English name, as prompts want it ("de" → "German"): the
/// languages Splashdown detects and offers.
pub fn english_name(code: &str) -> Option<&'static str> {
    Some(match code {
        "en" => "English",
        "de" => "German",
        "fr" => "French",
        "es" => "Spanish",
        "it" => "Italian",
        "pt" => "Portuguese",
        "nl" => "Dutch",
        "sv" => "Swedish",
        "da" => "Danish",
        "nb" => "Norwegian",
        "fi" => "Finnish",
        "pl" => "Polish",
        "cs" => "Czech",
        "sk" => "Slovak",
        "hu" => "Hungarian",
        "ro" => "Romanian",
        "bg" => "Bulgarian",
        "hr" => "Croatian",
        "el" => "Greek",
        "tr" => "Turkish",
        "ru" => "Russian",
        "uk" => "Ukrainian",
        "he" => "Hebrew",
        "ar" => "Arabic",
        "hi" => "Hindi",
        "th" => "Thai",
        "vi" => "Vietnamese",
        "id" => "Indonesian",
        "ca" => "Catalan",
        "zh" => "Chinese",
        "ja" => "Japanese",
        "ko" => "Korean",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_comes_down_to_its_language() {
        assert_eq!(base("de-DE"), "de");
        assert_eq!(base("pt_BR"), "pt");
        assert_eq!(base("EN"), "en");
        assert_eq!(base(""), "");
        assert_eq!(english_name("nb"), Some("Norwegian"));
    }
}
