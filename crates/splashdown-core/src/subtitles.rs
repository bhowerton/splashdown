//! Subtitles from a transcript: captions of a line or two, timed from the
//! words a local speech model placed, or estimated from a note's lines,
//! written as SRT or WebVTT.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// A word and when it was said, from the start of the recording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimedWord {
    /// As the model wrote it; may start with a space.
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

impl TimedWord {
    fn start(&self) -> Duration {
        Duration::from_millis(self.start_ms)
    }

    fn end(&self) -> Duration {
        Duration::from_millis(self.end_ms.max(self.start_ms))
    }
}

/// One caption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cue {
    pub start: Duration,
    pub end: Duration,
    /// One line, or two separated by `\n`.
    pub text: String,
}

/// Characters on a line, as subtitle guidelines have it.
const LINE: usize = 42;
/// Lines in a caption.
const LINES: usize = 2;
/// The longest a caption stays up.
const LONGEST: Duration = Duration::from_secs(7);
/// The shortest, when the next one leaves room.
const SHORTEST: Duration = Duration::from_secs(1);
/// A pause this long always starts a new caption.
const PAUSE: Duration = Duration::from_millis(1500);
/// Speaking speed, in characters a second, for estimating times.
const CHARS_PER_SEC: f64 = 15.0;

/// Captions from timed words, in order: broken at sentence ends, long
/// pauses, and before they'd grow past two lines or seven seconds. A cut
/// forced mid-sentence leaves at least a few words on each side, so a
/// sentence's last word isn't a caption of its own.
pub fn from_words(words: &[TimedWord]) -> Vec<Cue> {
    let spaced = words
        .iter()
        .any(|w| w.text.starts_with(char::is_whitespace));
    let text_of = |group: &[&TimedWord]| {
        group
            .iter()
            .fold(String::new(), |text, w| join(&text, &w.text, spaced))
    };
    // Groups of words, and whether each was cut short (not at a sentence
    // end, a clause or a pause).
    let mut groups: Vec<(Vec<&TimedWord>, bool)> = Vec::new();
    let mut group: Vec<&TimedWord> = Vec::new();
    for word in words.iter().filter(|w| !w.text.trim().is_empty()) {
        if let (Some(first), Some(last)) = (group.first(), group.last()) {
            let joined = join(&text_of(&group), &word.text, spaced);
            let paused = word.start().saturating_sub(last.end()) >= PAUSE;
            let too_long = joined.trim().chars().count() > LINE * LINES;
            let too_slow = word.end().saturating_sub(first.start()) > LONGEST;
            if paused || too_long || too_slow {
                groups.push((std::mem::take(&mut group), !paused));
            }
        }
        group.push(word);
        let len = text_of(&group).trim().chars().count();
        let ends_sentence = word.text.trim_end().ends_with(SENTENCE_ENDS);
        let ends_clause = word.text.trim_end().ends_with([',', ';', ':', '，', '、']);
        // A sentence end closes a caption that has some length; a comma one
        // that's most of the way full.
        if (ends_sentence && len >= LINE / 2) || (ends_clause && len >= LINE * LINES * 2 / 3) {
            groups.push((std::mem::take(&mut group), false));
        }
    }
    if !group.is_empty() {
        groups.push((group, false));
    }
    // After a forced cut, move words back across it until the next caption
    // has a few, while this one keeps a few too.
    for i in 1..groups.len() {
        if !groups[i - 1].1 {
            continue;
        }
        while groups[i].0.len() < FEWEST_WORDS && groups[i - 1].0.len() > FEWEST_WORDS {
            let Some(word) = groups[i - 1].0.pop() else {
                break;
            };
            groups[i].0.insert(0, word);
        }
    }
    let mut cues: Vec<Cue> = groups
        .iter()
        .filter_map(|(group, _)| {
            let (first, last) = (group.first()?, group.last()?);
            let text = text_of(group);
            let said = text.trim();
            (!said.is_empty()).then(|| Cue {
                start: first.start(),
                end: last.end(),
                text: wrap(said),
            })
        })
        .collect();
    settle(&mut cues, None);
    cues
}

/// Words a caption keeps at least, after a cut forced mid-sentence.
const FEWEST_WORDS: usize = 3;

/// A line of a note's transcript: when it starts, who said it ("" when the
/// note doesn't say) and what was said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Line<'a> {
    pub start: Duration,
    /// Where its speech ends, when known (a transcribed file's pieces);
    /// otherwise it runs until the next line or as long as it takes to say.
    pub end: Option<Duration>,
    pub who: &'a str,
    pub said: &'a str,
}

/// Captions estimated from a note's lines, when there are no word timings
/// (cloud speech, meetings, older transcripts). Each line runs until the
/// next one starts, or as long as it takes to say, whichever is sooner;
/// its text is cut into captions sharing that time by length. A new
/// speaker's name starts their first caption. `length` is the recording's,
/// if known.
pub fn from_lines(lines: &[Line<'_>], length: Option<Duration>) -> Vec<Cue> {
    let mut cues = Vec::new();
    let mut previous_who = "";
    for (i, line) in lines.iter().enumerate() {
        let said = line.said.trim();
        if said.is_empty() {
            continue;
        }
        let who = line.who.trim();
        let said = if !who.is_empty() && who != previous_who {
            format!("{who}: {said}")
        } else {
            said.to_string()
        };
        previous_who = who;
        let next = lines.get(i + 1).map(|l| l.start).or(length);
        let speaking = Duration::from_secs_f64(said.chars().count() as f64 / CHARS_PER_SEC);
        let span = match (line.end, next) {
            (Some(end), next) if end > line.start => {
                (end - line.start).min(next.map_or(Duration::MAX, |n| n.saturating_sub(line.start)))
            }
            (_, Some(next)) if next > line.start => (next - line.start).min(speaking.max(SHORTEST)),
            _ => speaking.max(SHORTEST),
        };
        let pieces = split(&said);
        let total: usize = pieces
            .iter()
            .map(|p| p.chars().count())
            .sum::<usize>()
            .max(1);
        let mut at = line.start;
        for piece in pieces {
            let share = span.mul_f64(piece.chars().count() as f64 / total as f64);
            cues.push(Cue {
                start: at,
                end: at + share,
                text: wrap(&piece),
            });
            at += share;
        }
    }
    settle(&mut cues, length);
    cues
}

/// SubRip: numbered captions, `00:01:02,500 --> 00:01:05,000`.
pub fn srt(cues: &[Cue]) -> String {
    let mut out = String::new();
    for (i, cue) in cues.iter().enumerate() {
        out.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            i + 1,
            stamp(cue.start, ','),
            stamp(cue.end, ','),
            cue.text
        ));
    }
    out
}

/// WebVTT: `WEBVTT`, then captions timed `00:01:02.500 --> 00:01:05.000`.
pub fn vtt(cues: &[Cue]) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for cue in cues {
        out.push_str(&format!(
            "{} --> {}\n{}\n\n",
            stamp(cue.start, '.'),
            stamp(cue.end, '.'),
            // "-->" would end the timing line's meaning early in a reader.
            cue.text.replace("-->", "→")
        ));
    }
    out
}

const SENTENCE_ENDS: [char; 6] = ['.', '?', '!', '。', '？', '！'];

/// `hh:mm:ss<sep>mmm`.
fn stamp(at: Duration, separator: char) -> String {
    let ms = at.as_millis();
    format!(
        "{:02}:{:02}:{:02}{separator}{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

/// `text` with `word` after it. Models that mark word starts with a space
/// (`spaced`) are joined as written; others get a space between words,
/// except between characters of scripts written without spaces.
fn join(text: &str, word: &str, spaced: bool) -> String {
    if spaced || text.is_empty() {
        return format!("{text}{word}");
    }
    let word = word.trim();
    let unspaced = |c: Option<char>| c.is_some_and(written_without_spaces);
    if unspaced(text.chars().last()) && unspaced(word.chars().next()) {
        format!("{text}{word}")
    } else {
        format!("{text} {word}")
    }
}

/// Chinese, Japanese and Thai characters, written without spaces.
fn written_without_spaces(c: char) -> bool {
    matches!(c as u32,
        0x3000..=0x30FF   // CJK punctuation, hiragana, katakana
        | 0x3400..=0x4DBF // CJK extension A
        | 0x4E00..=0x9FFF // CJK ideographs
        | 0xFF00..=0xFFEF // full-width forms
        | 0x0E00..=0x0E7F // Thai
    )
}

/// `text` cut into caption-sized pieces: at sentence ends where it can be,
/// then at commas, then at spaces.
fn split(text: &str) -> Vec<String> {
    let most = LINE * LINES;
    let mut pieces = Vec::new();
    let mut rest = text.trim();
    while rest.chars().count() > most {
        let head: String = rest.chars().take(most).collect();
        // The last good place to cut within the head, preferring a sentence
        // end past its first third, then a comma past half, then a space.
        let (third, half) = (head.len() / 3, head.len() / 2);
        let cut = last_after(&head, &SENTENCE_ENDS, third)
            .or_else(|| last_after(&head, &[',', ';', ':', '，', '、'], half))
            .or_else(|| head.rfind(' ').filter(|&i| i > 0))
            .unwrap_or(head.len());
        let (piece, after) = rest.split_at(cut);
        pieces.push(piece.trim().to_string());
        rest = after.trim_start();
    }
    if !rest.is_empty() {
        pieces.push(rest.to_string());
    }
    // Not a last word on its own: words move over from the piece before.
    if let [.., before, last] = pieces.as_mut_slice() {
        while last.split_whitespace().count() < FEWEST_WORDS
            && before.split_whitespace().count() > FEWEST_WORDS
        {
            let Some((kept, moved)) = before.rsplit_once(' ') else {
                break;
            };
            *last = format!("{moved} {last}");
            *before = kept.to_string();
        }
    }
    pieces
}

/// Just past the last of `marks` at or after byte `from` in `text`.
fn last_after(text: &str, marks: &[char], from: usize) -> Option<usize> {
    text.char_indices()
        .filter(|&(i, c)| i >= from && marks.contains(&c))
        .map(|(i, c)| i + c.len_utf8())
        .next_back()
}

/// A caption as one line, or two of about equal length broken at a space.
fn wrap(text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= LINE {
        return text;
    }
    let middle = text.len() / 2;
    let spaces = text.match_indices(' ').map(|(i, _)| i);
    match spaces.min_by_key(|&i| i.abs_diff(middle)) {
        Some(i) => format!("{}\n{}", &text[..i], &text[i + 1..]),
        // One long word, or a script without spaces: leave it to the player.
        None => text,
    }
}

/// Times a player can show: each caption up at least a second where the
/// next leaves room, never overlapping the next, never past `length`.
fn settle(cues: &mut [Cue], length: Option<Duration>) {
    for i in 0..cues.len() {
        let next = cues.get(i + 1).map(|c| c.start).or(length);
        let cue = &mut cues[i];
        if cue.end < cue.start + SHORTEST {
            cue.end = cue.start + SHORTEST;
        }
        if let Some(next) = next
            && cue.end > next
        {
            cue.end = next.max(cue.start + Duration::from_millis(1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(text: &str, start_ms: u64, end_ms: u64) -> TimedWord {
        TimedWord {
            text: text.into(),
            start_ms,
            end_ms,
        }
    }

    fn timed(sentence: &str, start_ms: u64, ms_per_word: u64) -> Vec<TimedWord> {
        sentence
            .split(' ')
            .enumerate()
            .map(|(i, t)| {
                let at = start_ms + i as u64 * ms_per_word;
                w(t, at, at + ms_per_word - 50)
            })
            .collect()
    }

    #[test]
    fn words_become_captions_at_sentence_ends_and_pauses() {
        let mut words = timed("Thanks everyone for joining the call today.", 0, 300);
        words.extend(timed("Let's start with the launch date.", 2400, 300));
        // A long pause, then a short reply.
        words.extend(timed("Sounds good.", 7000, 300));
        let cues = from_words(&words);
        let texts: Vec<&str> = cues.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "Thanks everyone for\njoining the call today.",
                "Let's start with the launch date.",
                "Sounds good."
            ]
        );
        assert_eq!(cues[0].start, Duration::ZERO);
        assert_eq!(cues[0].end, Duration::from_millis(2050));
        assert_eq!(cues[2].start, Duration::from_millis(7000));
        // Short, so up a second.
        assert_eq!(cues[2].end, Duration::from_millis(8000));
    }

    #[test]
    fn a_long_sentence_is_cut_before_two_lines_or_seven_seconds() {
        let sentence = "so what I was thinking is that we could move the whole \
                        launch back a week and use the extra time to finish the \
                        installer and the signing work before anyone sees it";
        let cues = from_words(&timed(sentence, 0, 250));
        assert!(cues.len() >= 2);
        for cue in &cues {
            assert!(cue.end - cue.start <= LONGEST + Duration::from_millis(1));
            let lines: Vec<&str> = cue.text.lines().collect();
            assert!(lines.len() <= LINES, "{:?}", cue.text);
            assert!(lines.iter().all(|l| l.chars().count() <= LINE + 10));
        }
        let all: Vec<String> = cues.iter().map(|c| c.text.replace('\n', " ")).collect();
        assert_eq!(all.join(" "), sentence);
    }

    #[test]
    fn a_sentence_cut_short_doesnt_leave_its_last_word_alone() {
        let sentence = "but we should talk about the budget for the second phase \
                        before we commit to anything.";
        let cues = from_words(&timed(sentence, 0, 200));
        let last = cues.last().unwrap().text.replace('\n', " ");
        assert_eq!(last.split(' ').count(), FEWEST_WORDS, "{cues:?}");
        assert!(last.ends_with("anything."));
    }

    #[test]
    fn words_marked_with_spaces_are_joined_as_written() {
        let words = [w(" Hello", 0, 400), w(" there.", 400, 800)];
        assert_eq!(from_words(&words)[0].text, "Hello there.");
        let words = [
            w("会議", 0, 400),
            w("は", 400, 500),
            w("三時です。", 500, 1200),
        ];
        assert_eq!(from_words(&words)[0].text, "会議は三時です。");
    }

    #[test]
    fn lines_share_their_time_and_name_a_new_speaker() {
        let lines = [
            Line {
                start: Duration::from_secs(0),
                end: None,
                who: "Them",
                said: "Hi, can you hear me? I think the audio is working now.",
            },
            Line {
                start: Duration::from_secs(4),
                end: None,
                who: "Them",
                said: "Okay, good.",
            },
            Line {
                start: Duration::from_secs(10),
                end: None,
                who: "You",
                said: "Yes, loud and clear.",
            },
        ];
        let cues = from_lines(&lines, Some(Duration::from_secs(12)));
        let texts: Vec<&str> = cues.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "Them: Hi, can you hear me? I\nthink the audio is working now.",
                "Okay, good.",
                "You: Yes, loud and clear."
            ]
        );
        // The first runs until the second starts; "Okay, good." for as
        // long as it takes to say (with a second at least), not the six
        // seconds until the next line.
        assert!(cues[0].end > Duration::from_secs(3) && cues[0].end <= Duration::from_secs(4));
        assert_eq!(cues[1].start, Duration::from_secs(4));
        assert_eq!(cues[1].end, Duration::from_secs(5));
        assert!(cues[2].end <= Duration::from_secs(12));
    }

    #[test]
    fn a_long_line_is_cut_into_captions_in_order() {
        let said = "We moved the launch from March third to March seventeenth because \
                    the Windows installer still fails signing. Anselm owns the fix, and \
                    the budget stays where it was. Marketing will hold the email.";
        let lines = [Line {
            start: Duration::from_secs(60),
            end: Some(Duration::from_secs(75)),
            who: "",
            said,
        }];
        let cues = from_lines(&lines, None);
        // Spread over where the speech was.
        assert_eq!(cues.last().unwrap().end, Duration::from_secs(75));
        assert!(cues.len() >= 3);
        assert_eq!(cues[0].start, Duration::from_secs(60));
        assert!(cues.windows(2).all(|p| p[0].end <= p[1].start));
        let all: Vec<String> = cues.iter().map(|c| c.text.replace('\n', " ")).collect();
        assert_eq!(
            all.join(" "),
            said.split_whitespace().collect::<Vec<_>>().join(" ")
        );
    }

    #[test]
    fn srt_and_vtt_are_written_as_players_read_them() {
        let cues = [
            Cue {
                start: Duration::from_millis(1500),
                end: Duration::from_millis(3250),
                text: "Hello there.".into(),
            },
            Cue {
                start: Duration::from_millis(3_723_004),
                end: Duration::from_millis(3_725_000),
                text: "An hour in.\nTwo lines.".into(),
            },
        ];
        assert_eq!(
            srt(&cues),
            "1\n00:00:01,500 --> 00:00:03,250\nHello there.\n\n\
             2\n01:02:03,004 --> 01:02:05,000\nAn hour in.\nTwo lines.\n\n"
        );
        assert_eq!(
            vtt(&cues),
            "WEBVTT\n\n00:00:01.500 --> 00:00:03.250\nHello there.\n\n\
             01:02:03.004 --> 01:02:05.000\nAn hour in.\nTwo lines.\n\n"
        );
    }
}
