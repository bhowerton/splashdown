//! Usage numbers for Settings → Insights: words dictated, time spent
//! speaking and the apps they went to, per day, in `insights.json` next to
//! the settings. Never what was said, and never sent anywhere.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Days, Local, NaiveDate};
use serde::{Deserialize, Serialize};

/// Days of history to keep.
const KEEP_DAYS: usize = 400;
/// Typing speed that "time saved" is measured against (words per minute).
pub const TYPING_WPM: f64 = 40.0;
/// Days shown in the chart.
const CHART_DAYS: u64 = 30;
const TOP_APPS: usize = 5;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Day {
    words: u64,
    dictations: u32,
    speaking_ms: u64,
    /// Words by app name.
    apps: BTreeMap<String, u64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Ledger {
    /// "2026-10-30" → totals.
    days: BTreeMap<String, Day>,
}

pub struct Insights {
    path: PathBuf,
    ledger: Mutex<Ledger>,
}

/// The app's record (one per user).
pub fn shared() -> &'static Insights {
    static INSIGHTS: OnceLock<Insights> = OnceLock::new();
    INSIGHTS.get_or_init(|| {
        Insights::open(
            splashdown_config::Paths::resolve()
                .map(|p| p.config_dir.join("insights.json"))
                .unwrap_or_else(|| std::env::temp_dir().join("splashdown-insights.json")),
        )
    })
}

/// What Settings shows.
#[derive(Debug, Serialize, PartialEq)]
pub struct Summary {
    pub today_words: u64,
    pub week_words: u64,
    pub total_words: u64,
    pub total_dictations: u64,
    /// Speaking speed over everything recorded (words per minute).
    pub speaking_wpm: Option<f64>,
    /// Typing the same words at `TYPING_WPM`, minus the time spent speaking.
    pub minutes_saved: f64,
    /// Consecutive days with a dictation, ending today or yesterday.
    pub streak_days: u32,
    /// Words per day for the last `CHART_DAYS` days, oldest first.
    pub chart: Vec<ChartDay>,
    /// The apps that got the most words in the last `CHART_DAYS` days.
    pub top_apps: Vec<(String, u64)>,
    pub typing_wpm: f64,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct ChartDay {
    pub date: String,
    pub words: u64,
}

fn key(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

impl Insights {
    pub fn open(path: PathBuf) -> Self {
        let ledger = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self {
            path,
            ledger: Mutex::new(ledger),
        }
    }

    fn ledger(&self) -> std::sync::MutexGuard<'_, Ledger> {
        self.ledger.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Counts one inserted dictation.
    pub fn record(&self, text: &str, speaking: Duration, app: Option<&str>, now: DateTime<Local>) {
        let words = text.split_whitespace().count() as u64;
        if words == 0 {
            return;
        }
        let mut ledger = self.ledger();
        let day = ledger.days.entry(key(now.date_naive())).or_default();
        day.words += words;
        day.dictations += 1;
        day.speaking_ms += speaking.as_millis() as u64;
        if let Some(app) = app.filter(|a| !a.is_empty()) {
            *day.apps.entry(app.to_string()).or_default() += words;
        }
        while ledger.days.len() > KEEP_DAYS {
            let oldest = ledger.days.keys().next().cloned().expect("non-empty");
            ledger.days.remove(&oldest);
        }
        self.save(&ledger);
    }

    pub fn clear(&self) {
        let mut ledger = self.ledger();
        ledger.days.clear();
        self.save(&ledger);
    }

    fn save(&self, ledger: &Ledger) {
        let json = serde_json::to_string_pretty(ledger).expect("insights serialize");
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&self.path, json) {
            tracing::warn!("couldn't save insights: {e}");
        }
    }

    pub fn summary(&self, now: DateTime<Local>) -> Summary {
        let ledger = self.ledger();
        let today = now.date_naive();
        let day = |d: NaiveDate| ledger.days.get(&key(d));
        let back = |n: u64| today.checked_sub_days(Days::new(n)).unwrap_or(today);

        let total_words: u64 = ledger.days.values().map(|d| d.words).sum();
        let total_ms: u64 = ledger.days.values().map(|d| d.speaking_ms).sum();
        let speaking_minutes = total_ms as f64 / 60_000.0;
        let week_words = (0..7).filter_map(|n| day(back(n))).map(|d| d.words).sum();

        // A streak may end yesterday: today isn't over yet.
        let start = if day(today).is_some() { 0 } else { 1 };
        let streak_days = (start..)
            .take_while(|&n| day(back(n)).is_some_and(|d| d.dictations > 0))
            .count() as u32;

        let chart = (0..CHART_DAYS)
            .rev()
            .map(|n| ChartDay {
                date: key(back(n)),
                words: day(back(n)).map_or(0, |d| d.words),
            })
            .collect();
        let mut apps: BTreeMap<&str, u64> = BTreeMap::new();
        for n in 0..CHART_DAYS {
            for (app, words) in day(back(n)).map(|d| &d.apps).into_iter().flatten() {
                *apps.entry(app).or_default() += words;
            }
        }
        let mut top_apps: Vec<(String, u64)> =
            apps.into_iter().map(|(a, w)| (a.to_string(), w)).collect();
        top_apps.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        top_apps.truncate(TOP_APPS);

        Summary {
            today_words: day(today).map_or(0, |d| d.words),
            week_words,
            total_words,
            total_dictations: ledger.days.values().map(|d| u64::from(d.dictations)).sum(),
            speaking_wpm: (speaking_minutes > 0.1).then(|| total_words as f64 / speaking_minutes),
            minutes_saved: (total_words as f64 / TYPING_WPM - speaking_minutes).max(0.0),
            streak_days,
            chart,
            top_apps,
            typing_wpm: TYPING_WPM,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(d: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, d, 12, 0, 0).unwrap()
    }

    #[test]
    fn counts_words_speed_streak_and_apps() {
        let path =
            std::env::temp_dir().join(format!("splashdown-insights-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let i = Insights::open(path.clone());
        let words = |n: usize| vec!["word"; n].join(" ");
        i.record(&words(120), Duration::from_secs(60), Some("Slack"), at(28));
        i.record(&words(30), Duration::from_secs(15), Some("Mail"), at(29));
        i.record(&words(50), Duration::from_secs(25), Some("Slack"), at(30));
        i.record("", Duration::from_secs(5), Some("Slack"), at(30));

        let s = i.summary(at(30));
        assert_eq!((s.today_words, s.week_words, s.total_words), (50, 200, 200));
        assert_eq!(s.total_dictations, 3);
        assert_eq!(s.streak_days, 3);
        // 200 words in 100 s.
        assert!((s.speaking_wpm.unwrap() - 120.0).abs() < 1e-9);
        // Typing 200 words at 40 wpm is 5 min; speaking took 1⅔.
        assert!((s.minutes_saved - (5.0 - 100.0 / 60.0)).abs() < 1e-9);
        assert_eq!(s.top_apps[0], ("Slack".to_string(), 170));
        assert_eq!(s.chart.len(), 30);
        assert_eq!(s.chart.last().unwrap().words, 50);

        // Tomorrow the streak still counts; the day after, it's broken.
        assert_eq!(i.summary(at(31)).streak_days, 3);
        assert_eq!(
            i.summary(Local.with_ymd_and_hms(2026, 11, 1, 12, 0, 0).unwrap())
                .streak_days,
            0
        );

        // Kept on disk.
        assert_eq!(Insights::open(path.clone()).summary(at(30)), s);
        i.clear();
        assert_eq!(i.summary(at(30)).total_words, 0);
        let _ = std::fs::remove_file(&path);
    }
}
