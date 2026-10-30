//! "Send feedback": a GitHub bug report prefilled through the issue form's
//! query parameters (field ids from `.github/ISSUE_TEMPLATE/bug_report.yml`).
//! Only the version, OS and provider names go in: never keys, dictated
//! text or file paths.

const NEW_ISSUE: &str = "https://github.com/bhowerton/splashdown/issues/new";

/// The OS as the template's "Operating system" dropdown names it.
const OS_NAME: &str = if cfg!(target_os = "macos") {
    "macOS"
} else {
    "Windows"
};

/// The new-issue address with `version`, the OS and `providers` filled in.
pub fn issue_url(version: &str, os_version: Option<&str>, providers: &str) -> String {
    let versions = match os_version {
        Some(v) => format!("Splashdown {version}, {OS_NAME} {v}"),
        None => format!("Splashdown {version}, {OS_NAME}"),
    };
    let params = [
        ("template", "bug_report.yml"),
        ("os", OS_NAME),
        ("versions", versions.as_str()),
        ("providers", providers),
    ];
    match tauri::Url::parse_with_params(NEW_ISSUE, params) {
        Ok(url) => url.into(),
        Err(_) => format!("{NEW_ISSUE}?template=bug_report.yml"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_encoded() {
        let url = issue_url(
            "0.2.0",
            Some("26.0.1"),
            "Speech to text: Apple on-device · Cleanup: OpenRouter · a/b&c=d",
        );
        assert!(url.starts_with(NEW_ISSUE));
        let parsed = tauri::Url::parse(&url).unwrap();
        let pairs: Vec<(String, String)> = parsed.query_pairs().into_owned().collect();
        let get = |k: &str| pairs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("template"), Some("bug_report.yml"));
        assert_eq!(get("os"), Some(OS_NAME));
        assert_eq!(
            get("versions"),
            Some(format!("Splashdown 0.2.0, {OS_NAME} 26.0.1").as_str())
        );
        assert_eq!(
            get("providers"),
            Some("Speech to text: Apple on-device · Cleanup: OpenRouter · a/b&c=d")
        );
        // Nothing that could split the query or the shell command.
        let query = parsed.query().unwrap();
        assert!(!query.contains(' ') && !query.contains("&c=d"));
    }

    #[test]
    fn unknown_os_version() {
        let url = issue_url("0.2.0", None, "x");
        let parsed = tauri::Url::parse(&url).unwrap();
        assert!(
            parsed
                .query_pairs()
                .any(|(k, v)| k == "versions" && v == format!("Splashdown 0.2.0, {OS_NAME}"))
        );
    }
}
