use std::io::{self, Read};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::TimeZone as _;

use serde_json::Value;

const SEP: &str = "\x1b[2m │ \x1b[0m";
const BAR_WIDTH: usize = 10;
const FIVE_HOUR_SECS: u64 = 5 * 3600;
const SEVEN_DAY_SECS: u64 = 7 * 24 * 3600;
const PACE_5H_MIN_ELAPSED: u64 = 30 * 60;
const PACE_7D_MIN_ELAPSED: u64 = 24 * 3600;

// Clamp guarantees the value is in [0, 100] before the cast, making both
// sign-loss and truncation impossible at runtime.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn pct_u32(f: f64) -> u32 {
    f.round().clamp(0.0, 100.0) as u32
}

fn color_by_used(val: u32) -> &'static str {
    if val >= 90 {
        "\x1b[31m"
    } else if val >= 50 {
        "\x1b[33m"
    } else {
        "\x1b[32m"
    }
}

fn mini_bar(percent: u32) -> String {
    let filled = ((percent as usize) * BAR_WIDTH / 100).min(BAR_WIDTH);
    let mut s = String::with_capacity(BAR_WIDTH * 3);
    for _ in 0..filled {
        s.push('▰');
    }
    for _ in filled..BAR_WIDTH {
        s.push('▱');
    }
    s
}

fn fmt_ctx_size(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{}M", tokens / 1_000_000)
    } else {
        format!("{}k", tokens / 1_000)
    }
}

fn now_secs() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

fn fmt_clock_time(unix_secs: u64) -> String {
    let Ok(secs_i64) = i64::try_from(unix_secs) else {
        return "--:--".to_string();
    };
    chrono::Local
        .timestamp_opt(secs_i64, 0)
        .single()
        .map_or_else(|| "--:--".to_string(), |dt| dt.format("%H:%M").to_string())
}

// Returns a pace indicator string after 30 min of elapsed window time.
// projected = used% / elapsed_fraction; green <90%, yellow 90-100%, red >100%.
// use_clock_time: when true and projected >100%, shows exhaustion as HH:MM; otherwise ⏳-Xm.
#[allow(clippy::cast_precision_loss)]
fn fmt_pace(used_pct: f64, resets_at: u64, now: u64, window_secs: u64, min_elapsed: u64, use_clock_time: bool) -> Option<String> {
    let window_start = resets_at.checked_sub(window_secs)?;
    let elapsed = now.checked_sub(window_start)?;

    if !(min_elapsed..window_secs).contains(&elapsed) {
        return None;
    }

    let projected = used_pct / (elapsed as f64 / window_secs as f64);

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let pct = projected.round() as u32;

    let (color, symbol) = if pct > 100 {
        ("\x1b[31m", "⚡")  // red  — will exceed
    } else if pct >= 90 {
        ("\x1b[33m", "⚠")  // yellow — approaching
    } else {
        ("\x1b[32m", "✓")  // green  — sustainable
    };

    let early_suffix = if pct > 100 {
        let time_to_exhaust_secs = (elapsed as f64) * 100.0 / used_pct;
        if use_clock_time {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let exhaust_unix = window_start + time_to_exhaust_secs.round() as u64;
            format!(" ({})", fmt_clock_time(exhaust_unix))
        } else {
            let secs_early = (window_secs as f64 - time_to_exhaust_secs).max(0.0);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let mins_early = (secs_early / 60.0).round() as u64;
            let h = mins_early / 60;
            let m = mins_early % 60;
            if h > 0 {
                format!(" ⏳-{h}h {m}m")
            } else {
                format!(" ⏳-{m}m")
            }
        }
    } else {
        String::new()
    };

    Some(format!("{color}{symbol} →{pct}%{early_suffix}\x1b[0m"))
}

fn main() {
    let mut input = String::new();
    let _ = io::stdin().read_to_string(&mut input);

    let v: Value = serde_json::from_str(&input).unwrap_or(Value::Null);

    let model = v["model"]["display_name"].as_str().unwrap_or("");
    let effort = v["effort"]["level"].as_str();
    let ctx_used = v["context_window"]["used_percentage"].as_f64();
    let ctx_size = v["context_window"]["context_window_size"].as_u64();
    let five_h_used = v["rate_limits"]["five_hour"]["used_percentage"].as_f64();
    let five_h_resets_at = v["rate_limits"]["five_hour"]["resets_at"].as_u64();
    let seven_d_used = v["rate_limits"]["seven_day"]["used_percentage"].as_f64();
    let seven_d_resets_at = v["rate_limits"]["seven_day"]["resets_at"].as_u64();
    let cost = v["cost"]["total_cost_usd"].as_f64();

    let now = now_secs().unwrap_or(0);
    let mut parts: Vec<String> = Vec::new();

    if !model.is_empty() {
        let label = effort
            .map_or_else(|| model.to_string(), |lvl| format!("{model} ({lvl})"));
        parts.push(format!("\x1b[1;35m{label}\x1b[0m"));
    }

    if let Some(ctx) = ctx_used {
        let val = pct_u32(ctx);
        let size = ctx_size.map_or(String::new(), |n| format!(" ({})", fmt_ctx_size(n)));
        parts.push(format!(
            "{}ctx {} {val}%{size}\x1b[0m",
            color_by_used(val),
            mini_bar(val),
        ));
    }

    if let Some(used) = five_h_used {
        let val = pct_u32(used);
        let reset = five_h_resets_at
            .map(|r| format!(" ({})", fmt_clock_time(r)))
            .unwrap_or_default();
        let pace = five_h_resets_at
            .and_then(|r| fmt_pace(used, r, now, FIVE_HOUR_SECS, PACE_5H_MIN_ELAPSED, true))
            .map_or(String::new(), |p| format!(" {p}"));
        parts.push(format!("{}⏰ 5h:{val}%{reset}\x1b[0m{pace}", color_by_used(val)));
    }

    if let Some(used) = seven_d_used {
        let val = pct_u32(used);
        let pace = seven_d_resets_at
            .and_then(|r| fmt_pace(used, r, now, SEVEN_DAY_SECS, PACE_7D_MIN_ELAPSED, false))
            .map_or(String::new(), |p| format!(" {p}"));
        parts.push(format!("{}⏰ 7d:{val}%\x1b[0m{pace}", color_by_used(val)));
    }

    if let Some(c) = cost {
        if c > 0.0 {
            parts.push(format!("\x1b[2m$ {c:.2}\x1b[0m"));
        }
    }

    print!("{}", parts.join(SEP));
}
