//! Em and en dashes in dictated text. Speech doesn't contain them, so one in
//! the cleaned text is the model's: it reads as machine-written. A safety
//! net for plain dictation, for when the prompt's rule isn't followed.

/// True if the speaker asked for a dash ("dash", "em dash", "en dash").
fn dictated(transcript: &str) -> bool {
    transcript
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| w.eq_ignore_ascii_case("dash") || w.eq_ignore_ascii_case("dashes"))
}

fn is_dash(c: char) -> bool {
    matches!(c, '\u{2014}' | '\u{2013}')
}

/// `output` with em and en dashes replaced, unless the transcript had one
/// or the speaker said "dash". Between words a dash becomes a comma; an en
/// dash between digits becomes a hyphen; one at the end of the text is
/// dropped. Code spans (backticks) and URLs are left alone.
pub fn remove_unspoken(transcript: &str, output: &str) -> String {
    if !output.contains(is_dash) || transcript.contains(is_dash) || dictated(transcript) {
        return output.to_string();
    }
    let mut out = String::with_capacity(output.len());
    let mut in_code = false;
    for (i, line) in output.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        // A fenced block is code too.
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            out.push_str(line);
            continue;
        }
        if in_code {
            out.push_str(line);
        } else {
            out.push_str(&fix_line(line));
        }
    }
    out
}

fn fix_line(line: &str) -> String {
    if !line.contains(is_dash) {
        return line.to_string();
    }
    // Split into pieces: inline code spans and whitespace-separated
    // tokens that look like URLs stay as they are.
    let mut out = String::new();
    let mut rest = line;
    let mut protected = false;
    while !rest.is_empty() {
        match rest.find('`') {
            Some(at) => {
                let (before, after) = rest.split_at(at);
                push_piece(&mut out, before, protected);
                out.push('`');
                protected = !protected;
                rest = &after[1..];
            }
            None => {
                push_piece(&mut out, rest, protected);
                break;
            }
        }
    }
    tidy(&out)
}

fn push_piece(out: &mut String, piece: &str, protected: bool) {
    if protected {
        // Marked so `tidy` leaves it alone: dashes inside code are swapped
        // for placeholders and put back afterwards.
        out.push_str(
            &piece
                .replace('\u{2014}', "\u{e000}")
                .replace('\u{2013}', "\u{e001}"),
        );
        return;
    }
    let chars: Vec<char> = piece.chars().collect();
    let mut token = String::new();
    let flush = |token: &mut String, out: &mut String| {
        if token.contains("://") || token.starts_with("www.") {
            out.push_str(
                &token
                    .replace('\u{2014}', "\u{e000}")
                    .replace('\u{2013}', "\u{e001}"),
            );
        } else {
            out.push_str(token);
        }
        token.clear();
    };
    // Tokens are cut at whitespace so a URL is judged whole.
    for (i, &c) in chars.iter().enumerate() {
        token.push(c);
        if c.is_whitespace() || i + 1 == chars.len() {
            flush(&mut token, out);
        }
    }
}

/// The replacement itself, on a line whose protected dashes are placeholders.
fn tidy(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if !is_dash(c) {
            out.push(c);
            i += 1;
            continue;
        }
        let prev = out.chars().rev().find(|c| !c.is_whitespace());
        let next = chars[i + 1..].iter().find(|c| !c.is_whitespace());
        let tight = !out.ends_with(char::is_whitespace)
            && chars.get(i + 1).is_some_and(|c| !c.is_whitespace());
        match (prev, next) {
            // A range or a minus between digits: "3–5".
            (Some(p), Some(n))
                if tight && c == '\u{2013}' && p.is_ascii_digit() && n.is_ascii_digit() =>
            {
                out.push('-');
                i += 1;
            }
            // Nothing before it (a leading dash): drop it.
            (None, _) => i = skip_space(&chars, i + 1),
            // Nothing after it (the end): drop it, and the space before.
            (_, None) => {
                while out.ends_with(char::is_whitespace) {
                    out.pop();
                }
                i = chars.len();
            }
            (Some(p), Some(n)) => {
                while out.ends_with(char::is_whitespace) {
                    out.pop();
                }
                // Already a mark there, or one coming: don't double it.
                let after = matches!(n, ',' | ';' | ':' | '.' | '!' | '?' | ')');
                if !after && !matches!(p, ',' | ';' | ':' | '.' | '!' | '?' | '(' | '-') {
                    out.push(',');
                }
                // Keep a space only where words follow on the same line.
                i = skip_space(&chars, i + 1);
                if !after {
                    out.push(' ');
                }
            }
        }
    }
    out.replace('\u{e000}', "\u{2014}")
        .replace('\u{e001}', "\u{2013}")
}

fn skip_space(chars: &[char], mut i: usize) -> usize {
    while chars.get(i).is_some_and(|c| c.is_whitespace()) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::remove_unspoken as fix;

    #[test]
    fn replaces_dashes_in_model_output() {
        let cases = [
            // between words
            (
                "x",
                "The office is great \u{2014} but the commute is long.",
                "The office is great, but the commute is long.",
            ),
            (
                "x",
                "The office is great\u{2014}but the commute is long.",
                "The office is great, but the commute is long.",
            ),
            ("x", "Fine \u{2013} let's go.", "Fine, let's go."),
            // no doubled commas, no comma before punctuation
            ("x", "Well, \u{2014} okay.", "Well, okay."),
            ("x", "Well \u{2014}, okay.", "Well, okay."),
            ("x", "Okay \u{2014}.", "Okay."),
            ("x", "Wait \u{2014}! Stop.", "Wait! Stop."),
            (
                "x",
                "Three things: \u{2014} a tent.",
                "Three things: a tent.",
            ),
            // en dash between digits
            ("x", "Open 9\u{2013}5 on weekdays.", "Open 9-5 on weekdays."),
            ("x", "Pages 3\u{2013}5.", "Pages 3-5."),
            // leading and trailing
            ("x", "\u{2014} Ben", "Ben"),
            ("x", "Count me in \u{2014}", "Count me in"),
            ("x", "Count me in\u{2014}", "Count me in"),
            ("x", "Count me in \u{2014}\n\nNext", "Count me in\n\nNext"),
            // lines stay lines
            (
                "x",
                "One \u{2014} two\nThree \u{2014} four",
                "One, two\nThree, four",
            ),
            // no dash, unchanged
            (
                "x",
                "Plain text, with a well-known hyphen.",
                "Plain text, with a well-known hyphen.",
            ),
        ];
        for (transcript, output, want) in cases {
            assert_eq!(fix(transcript, output), want, "{output:?}");
        }
    }

    #[test]
    fn leaves_code_and_urls_alone() {
        let cases = [
            "Run `a \u{2014} b` first.",
            "See https://example.com/a\u{2013}b for more.",
            "```\nx \u{2014} y\n```",
        ];
        for output in cases {
            assert_eq!(fix("x", output), output);
        }
        assert_eq!(
            fix("x", "Run `a \u{2014} b` first \u{2014} then stop."),
            "Run `a \u{2014} b` first, then stop."
        );
    }

    #[test]
    fn keeps_dashes_the_speaker_gave() {
        let output = "Ana \u{2014} Marco";
        // The transcript already had one.
        assert_eq!(fix("Ana \u{2013} Marco", output), output);
        assert_eq!(fix("Ana \u{2014} Marco", output), output);
        // Dictated.
        for said in [
            "put a dash between them",
            "an em dash here",
            "Use an en dash",
            "Ana DASH Marco",
        ] {
            assert_eq!(fix(said, output), output, "{said}");
        }
        // "dashboard" is not "dash".
        assert_eq!(fix("open the dashboard", output), "Ana, Marco");
    }
}
