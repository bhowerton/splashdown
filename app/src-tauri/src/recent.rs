//! What the user dictated in the last few minutes, kept in memory only and
//! sent with the next dictation into the same app, so the AI can tell what
//! "it" or "that" refers to and carry on an unfinished sentence (spec §6.1).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Older dictations aren't part of the same conversation any more.
const KEEP_FOR: Duration = Duration::from_secs(5 * 60);
/// At most this many earlier dictations go with one request…
const MAX_ITEMS: usize = 3;
/// …and at most this many characters of them, newest kept first.
const MAX_CHARS: usize = 1500;

struct Item {
    at: Instant,
    app_id: String,
    text: String,
}

#[derive(Default)]
pub struct Recent {
    items: VecDeque<Item>,
}

impl Recent {
    /// Remembers text just inserted into `app_id`.
    pub fn push(&mut self, app_id: &str, text: &str, now: Instant) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.items.push_back(Item {
            at: now,
            app_id: app_id.to_string(),
            text: text.to_string(),
        });
        self.prune(now);
    }

    /// Recent dictations into `app_id`, oldest first.
    pub fn for_app(&mut self, app_id: &str, now: Instant) -> Vec<String> {
        self.prune(now);
        let mut chars = 0;
        let mut out: Vec<String> = self
            .items
            .iter()
            .rev()
            .filter(|i| i.app_id == app_id)
            .take(MAX_ITEMS)
            .take_while(|i| {
                chars += i.text.chars().count();
                chars <= MAX_CHARS
            })
            .map(|i| i.text.clone())
            .collect();
        out.reverse();
        out
    }

    fn prune(&mut self, now: Instant) {
        while self
            .items
            .front()
            .is_some_and(|i| now.saturating_duration_since(i.at) > KEEP_FOR)
        {
            self.items.pop_front();
        }
        while self.items.len() > MAX_ITEMS * 4 {
            self.items.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_app_recent_and_oldest_first() {
        let t0 = Instant::now();
        let mut r = Recent::default();
        r.push("Slack", "first", t0);
        r.push("Mail", "elsewhere", t0);
        r.push("Slack", "second", t0 + Duration::from_secs(60));
        r.push("Slack", "  ", t0 + Duration::from_secs(61));
        assert_eq!(
            r.for_app("Slack", t0 + Duration::from_secs(120)),
            ["first", "second"]
        );
        // Five minutes after the first, only the second is left.
        assert_eq!(
            r.for_app("Slack", t0 + Duration::from_secs(301)),
            ["second"]
        );
        assert!(r.for_app("Notes", t0).is_empty());
    }

    #[test]
    fn newest_kept_within_the_limits() {
        let t0 = Instant::now();
        let mut r = Recent::default();
        for n in 1..=5 {
            r.push("Slack", &n.to_string(), t0);
        }
        assert_eq!(r.for_app("Slack", t0), ["3", "4", "5"]);

        let mut r = Recent::default();
        r.push("Slack", &"a".repeat(1000), t0);
        r.push("Slack", &"b".repeat(1000), t0);
        assert_eq!(r.for_app("Slack", t0), ["b".repeat(1000)]);
    }
}
