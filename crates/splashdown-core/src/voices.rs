//! Remembering people by voice across meetings: the voiceprints of people
//! the user named, and which of a new meeting's speakers clearly match one.
//! A voiceprint is a speaker model's summary of someone's speech (a vector
//! of 192 numbers, unit length); two of the same person score high.
//! Thresholds were measured on 40 speakers, phone-quality audio included:
//! no wrong names, no strangers named.

use serde::{Deserialize, Serialize};

/// The least score that can name someone.
pub const THRESHOLD: f32 = 0.55;
/// How far the best person must be ahead of the next one.
pub const MARGIN: f32 = 0.1;
/// Voiceprints kept per person (a headset, a laptop mic…), oldest dropped.
pub const MOST_PRINTS: usize = 5;
/// Speech needed from a speaker before they can be matched or remembered,
/// in seconds.
pub const LEAST_SPEECH_SECS: f32 = 20.0;

/// Someone the user named, by voice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Voice {
    pub name: String,
    /// Newest last.
    pub prints: Vec<Vec<f32>>,
    /// Meetings they were named or recognized in.
    #[serde(default)]
    pub meetings: u32,
}

/// Similarity of two unit-length voiceprints, -1 to 1.
pub fn similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return -1.0;
    }
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// `print` scaled to unit length (`None` if it's all zeros).
pub fn unit(print: Vec<f32>) -> Option<Vec<f32>> {
    let n = print.iter().map(|x| x * x).sum::<f32>().sqrt();
    (n > 0.0 && n.is_finite()).then(|| print.into_iter().map(|x| x / n).collect())
}

impl Voice {
    /// How much `print` sounds like this person: their closest print.
    pub fn score(&self, print: &[f32]) -> f32 {
        self.prints
            .iter()
            .map(|p| similarity(p, print))
            .fold(-1.0, f32::max)
    }
}

/// Adds `print` to `name`'s voice (a new one if there's none), keeping the
/// newest `MOST_PRINTS`.
pub fn learn(voices: &mut Vec<Voice>, name: &str, print: Vec<f32>) {
    let voice = match voices
        .iter()
        .position(|v| v.name.eq_ignore_ascii_case(name))
    {
        Some(i) => &mut voices[i],
        None => {
            voices.push(Voice {
                name: name.to_string(),
                prints: Vec::new(),
                meetings: 0,
            });
            voices.last_mut().expect("just added")
        }
    };
    voice.prints.push(print);
    if voice.prints.len() > MOST_PRINTS {
        voice.prints.remove(0);
    }
    voice.meetings += 1;
}

/// Takes `print` back from `name`'s voice (the name was a mistake), and
/// forgets them if it was their only one.
pub fn unlearn(voices: &mut Vec<Voice>, name: &str, print: &[f32]) {
    let Some(i) = voices
        .iter()
        .position(|v| v.name.eq_ignore_ascii_case(name))
    else {
        return;
    };
    let voice = &mut voices[i];
    let before = voice.prints.len();
    voice.prints.retain(|p| p.as_slice() != print);
    if voice.prints.len() == before {
        return;
    }
    voice.meetings = voice.meetings.saturating_sub(1);
    if voice.prints.is_empty() {
        voices.remove(i);
    }
}

/// Which of a meeting's speakers (label, voiceprint) are clearly someone
/// in `voices`: (label, name) pairs. A name needs a score of `THRESHOLD`,
/// `MARGIN` ahead of anyone else; each name goes to one speaker, best
/// first. When some of `invited` are saved, only they are considered.
pub fn recognize<'a>(
    speakers: &'a [(String, Vec<f32>)],
    voices: &[Voice],
    invited: &[String],
) -> Vec<(&'a str, String)> {
    let saved_invited: Vec<&Voice> = voices
        .iter()
        .filter(|v| invited.iter().any(|i| i.eq_ignore_ascii_case(&v.name)))
        .collect();
    let candidates: Vec<&Voice> = if saved_invited.is_empty() {
        voices.iter().collect()
    } else {
        saved_invited
    };
    // Each speaker's best person, if clearly ahead.
    let mut clear: Vec<(f32, &str, &str)> = Vec::new();
    for (label, print) in speakers {
        let mut scores: Vec<(f32, &str)> = candidates
            .iter()
            .map(|v| (v.score(print), v.name.as_str()))
            .collect();
        scores.sort_by(|a, b| b.0.total_cmp(&a.0));
        let Some(&(best, name)) = scores.first() else {
            continue;
        };
        let second = scores.get(1).map_or(-1.0, |s| s.0);
        if best >= THRESHOLD && best - second >= MARGIN {
            clear.push((best, label.as_str(), name));
        }
    }
    // One speaker per name: the best match keeps it.
    clear.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut out: Vec<(&str, String)> = Vec::new();
    for (_, label, name) in clear {
        if !out.iter().any(|(_, n)| n == name) {
            out.push((label, name.to_string()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit print near `axis` (a basis direction), pulled toward
    /// `toward` by `mix`.
    fn print(axis: usize, toward: usize, mix: f32) -> Vec<f32> {
        let mut v = vec![0.0; 8];
        v[axis] = 1.0 - mix;
        v[toward] += mix;
        unit(v).unwrap()
    }

    fn voice(name: &str, axis: usize) -> Voice {
        Voice {
            name: name.into(),
            prints: vec![print(axis, axis, 0.0)],
            meetings: 1,
        }
    }

    #[test]
    fn a_clear_match_is_named_and_a_stranger_isnt() {
        let voices = [voice("Priya", 0), voice("Sam", 1)];
        let speakers = vec![
            ("Speaker 1".to_string(), print(0, 2, 0.2)),
            ("Speaker 2".to_string(), print(5, 5, 0.0)),
        ];
        assert_eq!(
            recognize(&speakers, &voices, &[]),
            [("Speaker 1", "Priya".to_string())]
        );
    }

    #[test]
    fn someone_between_two_people_stays_unnamed() {
        let voices = [voice("Priya", 0), voice("Sam", 1)];
        // As close to Priya as to Sam.
        let speakers = vec![("Speaker 1".to_string(), print(0, 1, 0.5))];
        assert!(recognize(&speakers, &voices, &[]).is_empty());
    }

    #[test]
    fn a_name_goes_to_one_speaker_the_closest() {
        let voices = [voice("Priya", 0)];
        let speakers = vec![
            ("Speaker 1".to_string(), print(0, 3, 0.3)),
            ("Speaker 2".to_string(), print(0, 3, 0.1)),
        ];
        assert_eq!(
            recognize(&speakers, &voices, &[]),
            [("Speaker 2", "Priya".to_string())]
        );
    }

    #[test]
    fn saved_invitees_are_the_only_ones_considered() {
        let voices = [voice("Priya", 0), voice("Sam", 1)];
        let speakers = vec![("Speaker 1".to_string(), print(0, 0, 0.0))];
        // Priya sounds right, but only Sam was invited.
        assert!(recognize(&speakers, &voices, &["sam".into()]).is_empty());
        // Nobody invited is saved: everyone is considered.
        assert_eq!(recognize(&speakers, &voices, &["Dana".into()]).len(), 1);
    }

    #[test]
    fn learning_keeps_the_newest_prints() {
        let mut voices = Vec::new();
        for i in 0..7 {
            learn(&mut voices, "Priya", print(i % 8, i % 8, 0.0));
        }
        assert_eq!(voices.len(), 1);
        assert_eq!(voices[0].prints.len(), MOST_PRINTS);
        assert_eq!(voices[0].prints[0], print(2, 2, 0.0));
        assert_eq!(voices[0].meetings, 7);
    }

    #[test]
    fn a_voice_named_wrong_moves_to_the_right_name() {
        let mut voices = Vec::new();
        let p = print(0, 1, 0.0);
        learn(&mut voices, "Dan", p.clone());
        unlearn(&mut voices, "Dan", &p);
        learn(&mut voices, "Dana", p.clone());
        assert_eq!(voices.len(), 1);
        assert_eq!(voices[0].name, "Dana");
        // Names match whatever their case.
        learn(&mut voices, "dana", print(0, 2, 0.1));
        assert_eq!(voices.len(), 1);
        assert_eq!(voices[0].prints.len(), 2);
    }
}
