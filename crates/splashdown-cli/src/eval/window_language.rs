//! `eval window-language`: which language the conversation in a window is
//! in (eval/window-languages.toml), the detector the app uses against
//! detecting the whole window's text.

use super::*;

#[derive(Deserialize)]
pub(super) struct WindowFile {
    pub(super) case: Vec<WindowCase>,
}

#[derive(Deserialize)]
pub(super) struct WindowCase {
    pub(super) id: String,
    window: String,
    #[serde(default = "english")]
    spoken: String,
    /// A two-letter code, or "none".
    expect: String,
}

fn english() -> String {
    "en".into()
}

/// The whole window's text, as one: what a first try would do.
fn naive(window: &str, spoken: &str) -> Option<&'static str> {
    splashdown_core::aloud::language(window).filter(|l| *l != spoken)
}

pub(super) fn run(cases: &[&WindowCase], verbose: bool) {
    println!("Window language eval · {} cases\n", cases.len());
    let score = |name: &str, detect: &dyn Fn(&str, &str) -> Option<&'static str>| {
        let (mut right, mut wrong_switch, mut missed) = (0, 0, 0);
        for case in cases {
            let got = detect(&case.window, &case.spoken).unwrap_or("none");
            let ok = got == case.expect;
            if ok {
                right += 1;
            } else if got == "none" {
                missed += 1;
            } else {
                wrong_switch += 1;
            }
            if verbose || !ok {
                println!(
                    "{} {:<10} {:<42} expected {:<4} got {}",
                    if ok { "✓" } else { "✗" },
                    name,
                    case.id,
                    case.expect,
                    got
                );
            }
        }
        println!(
            "{name}: {right}/{} right · {wrong_switch} wrong switches · {missed} missed\n",
            cases.len()
        );
    };
    score("whole", &naive);
    score("messages", &|w, s| {
        splashdown_core::conversation::reply_language(w, s)
    });
}
