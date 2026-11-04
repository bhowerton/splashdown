//! A meeting's transcript from its two sides: **You** (the microphone) and
//! **Them** (what the computer played). Each side is transcribed in pieces
//! on its own; this puts the pieces in time order, joins a side's pieces
//! that follow on from each other into one paragraph, and drops what the
//! microphone only heard from the speakers (echo).
//!
//! Pieces arrive as they're transcribed, so a paragraph is only handed out
//! once nothing earlier can still come from either side.

use std::collections::VecDeque;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    You,
    Them,
}

impl Side {
    pub fn label(self) -> &'static str {
        match self {
            Side::You => "You",
            Side::Them => "Them",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// A transcribed piece of one side.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub side: Side,
    pub start: Duration,
    pub end: Duration,
    pub text: String,
}

/// What the transcript shows: one side speaking, from `start`.
#[derive(Debug, Clone, PartialEq)]
pub struct Paragraph {
    pub side: Side,
    pub start: Duration,
    pub text: String,
}

/// A side's next piece joins its paragraph when it starts this soon after
/// the paragraph's end...
const JOIN_GAP: Duration = Duration::from_secs(2);
/// ...and the paragraph isn't already this long (characters).
const MAX_PARAGRAPH: usize = 1200;
/// A microphone piece at least this long (words) is checked for echo;
/// "Yes." could really have been said while the other side spoke.
const ECHO_MIN_WORDS: usize = 3;
/// It's echo when this share of its words are in what the other side said
/// at the same time.
const ECHO_SHARE: f32 = 0.6;
/// The other side's pieces are kept this long, to check for echo.
const KEEP_THEM: Duration = Duration::from_secs(120);

#[derive(Default)]
pub struct Meeting {
    /// Transcribed pieces not yet placed, earliest first.
    pending: Vec<Segment>,
    /// How far each side has been transcribed, silent stretches included.
    reached: [Duration; 2],
    /// Recent pieces of the other side, for the echo check.
    them: VecDeque<Segment>,
    /// The paragraph still taking pieces.
    open: Option<(Paragraph, Duration)>,
    /// Pieces dropped as echo, for telling the user (headphones help).
    pub echoes: usize,
}

impl Meeting {
    pub fn new() -> Self {
        Self::default()
    }

    /// A transcribed piece. Empty text (silence) still counts as progress.
    pub fn push(&mut self, segment: Segment) {
        self.reached(segment.side, segment.end);
        if segment.text.trim().is_empty() {
            return;
        }
        let at = self.pending.partition_point(|s| s.start <= segment.start);
        self.pending.insert(at, segment);
    }

    /// `side` has been transcribed up to `end` (a piece with no speech).
    pub fn reached(&mut self, side: Side, end: Duration) {
        let r = &mut self.reached[side.index()];
        *r = (*r).max(end);
    }

    /// The paragraphs that are final: nothing earlier can come from either
    /// side any more.
    pub fn ready(&mut self) -> Vec<Paragraph> {
        let both = self.reached[0].min(self.reached[1]);
        let mut out = Vec::new();
        while self.pending.first().is_some_and(|s| s.end <= both) {
            let segment = self.pending.remove(0);
            self.place(segment, &mut out);
        }
        out
    }

    /// Everything that started before this has been handed out by `ready`
    /// (so it's in the note): the paragraph still taking pieces, and the
    /// pieces waiting for the other side, come after it.
    pub fn settled(&self) -> Duration {
        let both = self.reached[0].min(self.reached[1]);
        [
            Some(both),
            self.open.as_ref().map(|(p, _)| p.start),
            self.pending.first().map(|s| s.start),
        ]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or_default()
    }

    /// Everything left, once both sides have ended.
    pub fn finish(mut self) -> Vec<Paragraph> {
        let mut out = Vec::new();
        for segment in std::mem::take(&mut self.pending) {
            self.place(segment, &mut out);
        }
        out.extend(self.open.take().map(|(p, _)| p));
        out
    }

    fn place(&mut self, segment: Segment, out: &mut Vec<Paragraph>) {
        match segment.side {
            Side::Them => {
                while self
                    .them
                    .front()
                    .is_some_and(|t| t.end + KEEP_THEM < segment.start)
                {
                    self.them.pop_front();
                }
                self.them.push_back(segment.clone());
            }
            Side::You if self.is_echo(&segment) => {
                self.echoes += 1;
                return;
            }
            Side::You => {}
        }
        let text = segment.text.trim();
        if let Some((open, end)) = &mut self.open
            && open.side == segment.side
            && segment.start <= *end + JOIN_GAP
            && open.text.chars().count() < MAX_PARAGRAPH
        {
            open.text.push(' ');
            open.text.push_str(text);
            *end = segment.end;
            return;
        }
        out.extend(self.open.take().map(|(p, _)| p));
        self.open = Some((
            Paragraph {
                side: segment.side,
                start: segment.start,
                text: text.to_string(),
            },
            segment.end,
        ));
    }

    /// The microphone heard the speakers: most of its words are in what
    /// the other side said at the same time.
    fn is_echo(&self, you: &Segment) -> bool {
        let said = words(&you.text);
        if said.len() < ECHO_MIN_WORDS {
            return false;
        }
        let heard: Vec<String> = self
            .them
            .iter()
            .chain(self.pending.iter().filter(|s| s.side == Side::Them))
            .filter(|t| t.start < you.end && t.end > you.start)
            .flat_map(|t| words(&t.text))
            .collect();
        if heard.is_empty() {
            return false;
        }
        let shared = said.iter().filter(|w| heard.contains(w)).count();
        shared as f32 / said.len() as f32 >= ECHO_SHARE
    }
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// One turn of a transcript as a note holds it: `**[1:02] Them:** what
/// was said`, or `**[1:02]** what was said` when it doesn't say who (a
/// transcribed file, or a recording of one side). The one place this
/// format is written (`Turn::line`) and read (`Turn::parse`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Turn<'a> {
    /// When it was said, as written: "1:02" or "1:02:30".
    pub stamp: &'a str,
    /// Who said it; "" when the line doesn't say.
    pub who: &'a str,
    /// What was said.
    pub said: &'a str,
}

impl<'a> Turn<'a> {
    /// The turn on `line`, if it is one. Not any line that happens to
    /// start with `**[`, which the user may type in their own notes: the
    /// time must be m:ss or h:mm:ss, and a name has no `*` in it.
    pub fn parse(line: &'a str) -> Option<Self> {
        let (stamp, rest) = line.strip_prefix("**[")?.split_once(']')?;
        let parts: Vec<&str> = stamp.split(':').collect();
        let stamp_ok = (2..=3).contains(&parts.len())
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
            && parts[1..].iter().all(|p| p.len() == 2);
        if !stamp_ok {
            return None;
        }
        if let Some(said) = rest.strip_prefix("**") {
            return Some(Self {
                stamp,
                who: "",
                said: said.trim_start(),
            });
        }
        let (who, said) = rest.strip_prefix(' ')?.split_once(":**")?;
        (!who.is_empty() && !who.contains('*')).then(|| Self {
            stamp,
            who,
            said: said.trim_start(),
        })
    }

    /// When it was said, from the start of the recording.
    pub fn at(&self) -> Duration {
        let secs = self
            .stamp
            .split(':')
            .fold(0, |total, n| total * 60 + n.parse::<u64>().unwrap_or(0));
        Duration::from_secs(secs)
    }

    /// The turn as a line of the note.
    pub fn line(&self) -> String {
        if self.who.is_empty() {
            format!("**[{}]** {}", self.stamp, self.said)
        } else {
            format!("**[{}] {}:** {}", self.stamp, self.who, self.said)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turns_are_read_as_they_are_written() {
        for line in [
            "**[1:02:30] Them:** Hi there",
            "**[0:04]** Hi",
            "**[12:05] Dana Lee:** ok",
        ] {
            assert_eq!(Turn::parse(line).map(|t| t.line()).as_deref(), Some(line));
        }
        let turn = Turn::parse("**[1:02:30] Them:** Hi").unwrap();
        assert_eq!((turn.who, turn.said), ("Them", "Hi"));
        assert_eq!(turn.at(), Duration::from_secs(3750));
        // The user's own lines aren't turns.
        for line in [
            "**[x] call Sam",
            "**[0:4] You:** no",
            "**[1:00] *bold*:** x",
            "**[todo]** milk",
        ] {
            assert_eq!(Turn::parse(line), None, "{line}");
        }
    }

    fn seg(side: Side, start: u64, end: u64, text: &str) -> Segment {
        Segment {
            side,
            start: Duration::from_secs(start),
            end: Duration::from_secs(end),
            text: text.into(),
        }
    }

    fn lines(paragraphs: &[Paragraph]) -> Vec<String> {
        paragraphs
            .iter()
            .map(|p| format!("{} {}: {}", p.start.as_secs(), p.side.label(), p.text))
            .collect()
    }

    #[test]
    fn settled_is_where_the_unwritten_part_starts() {
        let mut m = Meeting::new();
        m.push(seg(Side::Them, 0, 9, "Can we move the launch?"));
        m.push(seg(Side::You, 10, 20, "Thursday works for me."));
        m.reached(Side::Them, Duration::from_secs(30));
        let written = m.ready();
        // Them's paragraph is written; You's is still open (it may grow).
        assert_eq!(lines(&written), ["0 Them: Can we move the launch?"]);
        assert_eq!(m.settled(), Duration::from_secs(10));
        // A piece waiting for the other side keeps it there too.
        m.push(seg(Side::Them, 32, 40, "Great."));
        m.push(seg(Side::You, 45, 50, "Bye."));
        m.reached(Side::Them, Duration::from_secs(41));
        m.ready();
        assert_eq!(m.settled(), Duration::from_secs(32));
    }

    #[test]
    fn sides_interleave_in_time_order() {
        let mut m = Meeting::new();
        // The microphone's pieces arrive first, but Them spoke first.
        m.push(seg(Side::You, 10, 20, "Thursday works for me."));
        m.push(seg(Side::You, 40, 50, "Great, see you then."));
        assert!(m.ready().is_empty(), "Them hasn't been transcribed yet");
        m.push(seg(Side::Them, 0, 9, "Can we move the launch?"));
        m.push(seg(Side::Them, 25, 35, "Thursday it is."));
        m.reached(Side::Them, Duration::from_secs(60));
        let mut out = m.ready();
        out.extend(m.finish());
        assert_eq!(
            lines(&out),
            [
                "0 Them: Can we move the launch?",
                "10 You: Thursday works for me.",
                "25 Them: Thursday it is.",
                "40 You: Great, see you then.",
            ]
        );
    }

    #[test]
    fn a_side_that_keeps_talking_is_one_paragraph() {
        let mut m = Meeting::new();
        m.push(seg(Side::Them, 0, 30, "First part."));
        m.push(seg(Side::Them, 31, 60, "Second part."));
        m.push(seg(Side::Them, 70, 80, "After a long pause."));
        m.reached(Side::You, Duration::from_secs(80));
        let mut out = m.ready();
        out.extend(m.finish());
        assert_eq!(
            lines(&out),
            [
                "0 Them: First part. Second part.",
                "70 Them: After a long pause."
            ]
        );
    }

    #[test]
    fn speakers_heard_by_the_microphone_are_dropped() {
        let mut m = Meeting::new();
        m.push(seg(
            Side::Them,
            0,
            20,
            "We should ship the Windows build on Friday after QA signs off.",
        ));
        // Echo: the same words, picked up a little later and garbled.
        m.push(seg(
            Side::You,
            1,
            21,
            "we should ship the windows build on friday after qa",
        ));
        // A real answer that shares a word or two isn't echo.
        m.push(seg(
            Side::You,
            22,
            30,
            "Friday is fine with me, I'll tell QA.",
        ));
        // Short replies are never checked.
        m.push(seg(Side::You, 31, 32, "Friday."));
        m.reached(Side::Them, Duration::from_secs(40));
        let mut out = m.ready();
        assert_eq!(m.echoes, 1);
        out.extend(m.finish());
        assert_eq!(
            lines(&out),
            [
                "0 Them: We should ship the Windows build on Friday after QA signs off.",
                "22 You: Friday is fine with me, I'll tell QA. Friday.",
            ]
        );
    }

    #[test]
    fn silence_moves_a_side_along() {
        let mut m = Meeting::new();
        m.push(seg(Side::You, 0, 10, "Anyone there?"));
        // The other side said nothing: an empty piece still says how far
        // it has got.
        m.push(seg(Side::Them, 0, 30, "  "));
        m.push(seg(Side::You, 40, 50, "Hello?"));
        m.reached(Side::Them, Duration::from_secs(60));
        assert_eq!(lines(&m.ready()), ["0 You: Anyone there?"]);
        assert_eq!(lines(&m.finish()), ["40 You: Hello?"]);
    }

    #[test]
    fn long_monologues_are_split() {
        let mut m = Meeting::new();
        let piece = "word ".repeat(150);
        for i in 0..4 {
            m.push(seg(Side::Them, i * 30, i * 30 + 29, &piece));
        }
        m.reached(Side::You, Duration::from_secs(200));
        let mut out = m.ready();
        out.extend(m.finish());
        assert!(out.len() >= 2, "{}", out.len());
        assert!(
            out.iter()
                .all(|p| p.text.len() < MAX_PARAGRAPH + piece.len())
        );
    }
}
