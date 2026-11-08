//! What cloud providers have cost, per day and per month, kept in
//! `spend.json` next to the settings (never sent anywhere). Feeds the
//! spending summary in Settings and the budget: over the limit, the engine
//! is rebuilt to use on-device models until the period ends.
//!
//! Costs are estimates (see `splashdown_providers::pricing`). A call whose
//! price isn't known is counted, not priced.

use splashdown_core::sync::Locked;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use splashdown_config::Budget;
use splashdown_core::Usage;

/// Months and days of history to keep.
const KEEP_MONTHS: usize = 13;
const KEEP_DAYS: usize = 62;
/// A warning is shown once a period passes this share of its limit.
pub const WARN_AT: f64 = 0.8;

/// Totals for one day or month.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Period {
    pub cost_usd: f64,
    /// Dictations (and other requests) that used a paid provider.
    pub requests: u32,
    /// Paid calls whose price wasn't known.
    pub unpriced: u32,
    /// Cost per provider name ("openai/gpt-4o-mini-transcribe", ...).
    pub by_provider: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Ledger {
    /// "2026-10" → totals.
    months: BTreeMap<String, Period>,
    /// "2026-10-29" → totals.
    days: BTreeMap<String, Period>,
}

/// Where spending stands against the budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BudgetState {
    /// No limit set, or comfortably under it.
    Fine,
    /// Past `WARN_AT` of a limit.
    Near,
    /// A limit is reached: on-device models only.
    Reached,
}

pub struct Spend {
    path: PathBuf,
    ledger: Mutex<Ledger>,
}

/// The app's ledger (one per user).
pub fn shared() -> &'static Spend {
    static SPEND: OnceLock<Spend> = OnceLock::new();
    SPEND.get_or_init(|| {
        Spend::open(
            splashdown_config::Paths::resolve()
                .map(|p| p.spend_file())
                .unwrap_or_else(|| std::env::temp_dir().join("splashdown-spend.json")),
        )
    })
}

/// Records what one request's providers cost, splitting a pipeline outcome
/// into speech-to-text and AI parts.
pub fn record_outcome(outcome: &splashdown_core::Outcome) {
    let stt = Usage {
        cost_usd: (outcome.usage.cost_usd - outcome.llm_usage.cost_usd).max(0.0),
        unpriced: outcome.usage.unpriced && !outcome.llm_usage.unpriced,
        ..Usage::FREE
    };
    let mut parts = Vec::new();
    if let Some(name) = &outcome.stt_name {
        parts.push((name.clone(), stt));
    }
    if let Some(name) = &outcome.llm_name {
        parts.push((name.clone(), outcome.llm_usage));
    }
    shared().record(&parts, Local::now());
}

fn month_key(now: DateTime<Local>) -> String {
    now.format("%Y-%m").to_string()
}

fn day_key(now: DateTime<Local>) -> String {
    now.format("%Y-%m-%d").to_string()
}

impl Spend {
    /// Loads `path`, starting empty if it's missing or unreadable.
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
        self.ledger.locked()
    }

    /// Adds one request's costs. `parts` names each provider and what it
    /// cost; free (on-device) parts are skipped.
    pub fn record(&self, parts: &[(String, Usage)], now: DateTime<Local>) {
        let paid: Vec<_> = parts
            .iter()
            .filter(|(_, u)| u.unpriced || u.cost_usd > 0.0)
            .collect();
        if paid.is_empty() {
            return;
        }
        let mut ledger = self.ledger();
        add(ledger.months.entry(month_key(now)).or_default(), &paid);
        add(ledger.days.entry(day_key(now)).or_default(), &paid);
        prune(&mut ledger.months, KEEP_MONTHS);
        prune(&mut ledger.days, KEEP_DAYS);
        let json = serde_json::to_string_pretty(&*ledger).expect("ledger serializes");
        drop(ledger);
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&self.path, json) {
            tracing::warn!("couldn't save spending: {e}");
        }
    }

    pub fn this_month(&self, now: DateTime<Local>) -> Period {
        self.ledger()
            .months
            .get(&month_key(now))
            .cloned()
            .unwrap_or_default()
    }

    pub fn today(&self, now: DateTime<Local>) -> Period {
        self.ledger()
            .days
            .get(&day_key(now))
            .cloned()
            .unwrap_or_default()
    }

    pub fn budget_state(&self, budget: &Budget, now: DateTime<Local>) -> BudgetState {
        let checks = [
            (budget.monthly_usd, self.this_month(now).cost_usd),
            (budget.daily_usd, self.today(now).cost_usd),
        ];
        let mut state = BudgetState::Fine;
        for (limit, spent) in checks {
            let Some(limit) = limit.filter(|l| *l > 0.0) else {
                continue;
            };
            if spent >= limit {
                return BudgetState::Reached;
            }
            if spent >= limit * WARN_AT {
                state = BudgetState::Near;
            }
        }
        state
    }
}

fn add(period: &mut Period, paid: &[&(String, Usage)]) {
    period.requests += 1;
    for (name, usage) in paid {
        period.cost_usd += usage.cost_usd;
        if usage.unpriced {
            period.unpriced += 1;
        }
        *period.by_provider.entry(name.clone()).or_default() += usage.cost_usd;
    }
}

/// Keeps the newest `keep` entries (keys sort by date).
fn prune(map: &mut BTreeMap<String, Period>, keep: usize) {
    while map.len() > keep {
        let oldest = map.keys().next().cloned().expect("non-empty");
        map.remove(&oldest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(y: i32, m: u32, d: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap()
    }

    fn temp_spend(name: &str) -> Spend {
        let path = std::env::temp_dir().join(format!(
            "splashdown-spend-{name}-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        Spend::open(path)
    }

    #[test]
    fn records_by_day_month_and_provider() {
        let s = temp_spend("record");
        let now = at(2026, 10, 29);
        s.record(
            &[
                ("openai/gpt-4o-mini-transcribe".into(), Usage::priced(0.003)),
                ("local/qwen3-4b".into(), Usage::FREE),
            ],
            now,
        );
        s.record(&[("custom/remote".into(), Usage::unknown())], now);
        let month = s.this_month(now);
        assert_eq!(month.requests, 2);
        assert_eq!(month.unpriced, 1);
        assert!((month.cost_usd - 0.003).abs() < 1e-12);
        assert!(
            !month.by_provider.contains_key("local/qwen3-4b"),
            "free parts skipped"
        );
        assert_eq!(s.today(now), month);
        assert_eq!(
            s.today(at(2026, 10, 30)),
            Period::default(),
            "a new day starts empty"
        );

        // It's on disk.
        let reopened = Spend::open(s.path.clone());
        assert_eq!(reopened.this_month(now), month);
        let _ = std::fs::remove_file(&s.path);
    }

    #[test]
    fn free_requests_are_not_counted() {
        let s = temp_spend("free");
        let now = at(2026, 10, 29);
        s.record(&[("apple/speech".into(), Usage::FREE)], now);
        assert_eq!(s.this_month(now), Period::default());
    }

    #[test]
    fn budget_warns_then_stops() {
        let s = temp_spend("budget");
        let now = at(2026, 10, 29);
        let monthly = Budget {
            monthly_usd: Some(1.0),
            daily_usd: None,
        };
        assert_eq!(s.budget_state(&monthly, now), BudgetState::Fine);
        s.record(&[("p".into(), Usage::priced(0.85))], now);
        assert_eq!(s.budget_state(&monthly, now), BudgetState::Near);
        s.record(&[("p".into(), Usage::priced(0.2))], now);
        assert_eq!(s.budget_state(&monthly, now), BudgetState::Reached);
        // A new month starts fresh.
        assert_eq!(s.budget_state(&monthly, at(2026, 11, 1)), BudgetState::Fine);
        // A daily limit is checked too; no limit means fine.
        let daily = Budget {
            monthly_usd: None,
            daily_usd: Some(0.5),
        };
        assert_eq!(s.budget_state(&daily, now), BudgetState::Reached);
        assert_eq!(s.budget_state(&Budget::default(), now), BudgetState::Fine);
        let _ = std::fs::remove_file(&s.path);
    }

    #[test]
    fn keeps_a_limited_history() {
        let mut map = BTreeMap::new();
        for d in 1..=70 {
            map.insert(format!("2026-01-{d:02}"), Period::default());
        }
        prune(&mut map, KEEP_DAYS);
        assert_eq!(map.len(), KEEP_DAYS);
        assert!(!map.contains_key("2026-01-01"));
    }
}
