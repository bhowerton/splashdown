//! Output formatting: the final text goes to stdout (so it can be piped);
//! diagnostics go to stderr.

use std::io::Write;

use splashdown_core::{Fallback, Outcome};

pub fn print(outcome: &Outcome) {
    eprintln!("── mode        {:?}", outcome.mode);
    eprintln!("── heard       {}", outcome.raw);
    if let Some(instruction) = &outcome.instruction {
        eprintln!("── instruction {instruction}");
    }
    if let Some(fallback) = &outcome.fallback {
        let why = match fallback {
            Fallback::Guard(reason) => reason.describe().to_string(),
            Fallback::ModelError(e) => format!("AI error: {e}"),
        };
        eprintln!("── fallback    used raw text ({why})");
    }
    let mut timing = Vec::new();
    if let Some(name) = &outcome.stt_name {
        timing.push(format!(
            "stt {:.2}s ({name})",
            outcome.stt_time.as_secs_f32()
        ));
    }
    if let Some(name) = &outcome.llm_name {
        timing.push(format!(
            "ai {:.2}s ({name})",
            outcome.llm_time.as_secs_f32()
        ));
    }
    if !timing.is_empty() {
        eprintln!("── timing      {}", timing.join(" · "));
    }
    eprintln!("──");
    println!("{}", outcome.text);
}

pub fn download_progress(done: u64, total: Option<u64>) {
    let mb = |b: u64| b as f64 / 1_048_576.0;
    match total {
        Some(t) => eprint!(
            "\r\x1b[2Kdownloading {:.0} / {:.0} MB ({:.0}%)",
            mb(done),
            mb(t),
            done as f64 * 100.0 / t as f64
        ),
        None => eprint!("\r\x1b[2Kdownloading {:.0} MB", mb(done)),
    }
    let _ = std::io::stderr().flush();
}
