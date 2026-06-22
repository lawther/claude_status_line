use std::io::{self, Read};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::TimeZone as _;

use serde_json::Value;

const SEP: &str = "\x1b[2m │ \x1b[0m";
const BAR_WIDTH: usize = 10;
const FIVE_HOUR_SECS: u64 = 5 * 3600;
const SEVEN_DAY_SECS: u64 = 7 * 24 * 3600;
// Show pace once either this fraction of the window has elapsed, or this much quota is used.
const PACE_MIN_PCT: f64 = 10.0;

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

// Returns a pace indicator string once PACE_MIN_PCT of the window has elapsed or PACE_MIN_PCT
// of the quota has been used, whichever comes first. Green <90%, yellow 90-100%, red >100%.
// use_clock_time: when true and projected >100%, shows exhaustion as HH:MM; otherwise ⏳-Xm.
#[allow(clippy::cast_precision_loss)]
fn fmt_pace(used_pct: f64, resets_at: u64, now: u64, window_secs: u64, use_clock_time: bool) -> Option<String> {
    let window_start = resets_at.checked_sub(window_secs)?;
    let elapsed = now.checked_sub(window_start)?;

    if elapsed >= window_secs {
        return None;
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let time_threshold = (window_secs as f64 * PACE_MIN_PCT / 100.0).round() as u64;
    if elapsed < time_threshold && used_pct < PACE_MIN_PCT {
        return None;
    }

    let projected = used_pct / (elapsed as f64 / window_secs as f64);

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let pct = projected.round() as u32;

    let (color, symbol) = if pct > 100 {
        ("\x1b[31m", "🔥")  // red  — will exceed
    } else if pct >= 90 {
        ("\x1b[33m", "⚠️")  // yellow — approaching
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
            .and_then(|r| fmt_pace(used, r, now, FIVE_HOUR_SECS, true))
            .map_or(String::new(), |p| format!(" {p}"));
        parts.push(format!("{}5h {} {val}%{reset}\x1b[0m{pace}", color_by_used(val), mini_bar(val)));
    }

    if let Some(used) = seven_d_used {
        let val = pct_u32(used);
        let pace = seven_d_resets_at
            .and_then(|r| fmt_pace(used, r, now, SEVEN_DAY_SECS, false))
            .map_or(String::new(), |p| format!(" {p}"));
        parts.push(format!("{}7d {} {val}%\x1b[0m{pace}", color_by_used(val), mini_bar(val)));
    }

    if let Some(c) = cost {
        if c > 0.0 {
            parts.push(format!("\x1b[2m$ {c:.2}\x1b[0m"));
        }
    }

    print!("{}", parts.join(SEP));
}

#[cfg(test)]
mod tests {
    use super::*;

    // Anchor: set resets_at = window_secs so window_start = 0 and elapsed = now directly.
    const ANCHOR_5H: u64 = FIVE_HOUR_SECS;
    const ANCHOR_7D: u64 = SEVEN_DAY_SECS;

    fn elapsed_pct(window_secs: u64, pct: f64) -> u64 {
        (window_secs as f64 * pct / 100.0).round() as u64
    }

    // --- pct_u32 ---

    #[test]
    fn rounds_percentage_down() {
        assert_eq!(pct_u32(72.4), 72);
    }

    #[test]
    fn rounds_percentage_up() {
        assert_eq!(pct_u32(72.5), 73);
    }

    #[test]
    fn clamps_percentage_below_zero() {
        assert_eq!(pct_u32(-5.0), 0);
    }

    #[test]
    fn clamps_percentage_above_one_hundred() {
        assert_eq!(pct_u32(105.0), 100);
    }

    // --- fmt_ctx_size ---

    #[test]
    fn formats_tokens_below_million_as_k() {
        assert_eq!(fmt_ctx_size(200_000), "200k");
        assert_eq!(fmt_ctx_size(999_999), "999k");
    }

    #[test]
    fn formats_tokens_at_or_above_million_as_m() {
        assert_eq!(fmt_ctx_size(1_000_000), "1M");
        assert_eq!(fmt_ctx_size(2_500_000), "2M");
    }

    // --- mini_bar ---

    #[test]
    fn bar_is_all_empty_at_zero_percent() {
        assert_eq!(mini_bar(0), "▱▱▱▱▱▱▱▱▱▱");
    }

    #[test]
    fn bar_is_all_filled_at_one_hundred_percent() {
        assert_eq!(mini_bar(100), "▰▰▰▰▰▰▰▰▰▰");
    }

    #[test]
    fn bar_is_half_filled_at_fifty_percent() {
        assert_eq!(mini_bar(50), "▰▰▰▰▰▱▱▱▱▱");
    }

    // --- fmt_pace: threshold triggering ---

    #[test]
    fn pace_suppressed_when_both_time_and_usage_below_threshold() {
        // 8% elapsed, 9% used — both below the 10% threshold
        let elapsed = elapsed_pct(FIVE_HOUR_SECS, 8.0);
        assert!(fmt_pace(9.0, ANCHOR_5H, elapsed, FIVE_HOUR_SECS, false).is_none());
    }

    #[test]
    fn pace_shown_when_usage_meets_threshold_before_time_does() {
        // 8% elapsed, 11% used — usage crosses 10% while time has not
        let elapsed = elapsed_pct(FIVE_HOUR_SECS, 8.0);
        assert!(fmt_pace(11.0, ANCHOR_5H, elapsed, FIVE_HOUR_SECS, false).is_some());
    }

    #[test]
    fn pace_shown_when_time_meets_threshold_before_usage_does() {
        // 11% elapsed, 9% used — time crosses 10% while usage has not
        let elapsed = elapsed_pct(FIVE_HOUR_SECS, 11.0);
        assert!(fmt_pace(9.0, ANCHOR_5H, elapsed, FIVE_HOUR_SECS, false).is_some());
    }

    #[test]
    fn pace_suppressed_when_window_is_complete() {
        // elapsed = window — nothing left to project
        assert!(fmt_pace(90.0, ANCHOR_5H, FIVE_HOUR_SECS, FIVE_HOUR_SECS, false).is_none());
    }

    #[test]
    fn pace_thresholds_apply_independently_to_seven_day_window() {
        // 8% elapsed, 9% used on 7d window — suppressed
        let elapsed = elapsed_pct(SEVEN_DAY_SECS, 8.0);
        assert!(fmt_pace(9.0, ANCHOR_7D, elapsed, SEVEN_DAY_SECS, false).is_none());
        // 11% elapsed — shown
        let elapsed = elapsed_pct(SEVEN_DAY_SECS, 11.0);
        assert!(fmt_pace(9.0, ANCHOR_7D, elapsed, SEVEN_DAY_SECS, false).is_some());
    }

    // --- fmt_pace: projected rate colouring ---
    // elapsed = 5000s (~28% of 5h) for all colour tests, keeping well past the threshold.

    #[test]
    fn pace_green_when_projected_under_ninety_percent() {
        // 20% used at 28% elapsed → projected ≈ 72%
        let result = fmt_pace(20.0, ANCHOR_5H, 5000, FIVE_HOUR_SECS, false).unwrap();
        assert!(result.contains('✓'), "expected ✓ in {result:?}");
    }

    #[test]
    fn pace_yellow_when_projected_between_ninety_and_one_hundred_percent() {
        // 26% used at 28% elapsed → projected ≈ 94%
        let result = fmt_pace(26.0, ANCHOR_5H, 5000, FIVE_HOUR_SECS, false).unwrap();
        assert!(result.contains("⚠️"), "expected ⚠️ in {result:?}");
    }

    #[test]
    fn pace_red_when_projected_over_one_hundred_percent() {
        // 30% used at 28% elapsed → projected ≈ 108%
        let result = fmt_pace(30.0, ANCHOR_5H, 5000, FIVE_HOUR_SECS, false).unwrap();
        assert!(result.contains("🔥"), "expected 🔥 in {result:?}");
    }

    // --- fmt_pace: exhaustion marker when projected > 100% ---

    #[test]
    fn pace_shows_duration_marker_when_exceeding_without_clock_time() {
        // 30% used at 5000s elapsed → time_to_exhaust = 5000 * 100/30 ≈ 16667s
        // secs_early = 18000 - 16667 ≈ 1333s → 22m
        let result = fmt_pace(30.0, ANCHOR_5H, 5000, FIVE_HOUR_SECS, false).unwrap();
        assert!(result.contains("⏳-22m"), "expected ⏳-22m in {result:?}");
    }

    #[test]
    fn pace_shows_clock_time_when_exceeding_with_clock_time_flag() {
        // Exact time is timezone-dependent; verify the shape "(HH:MM)" is present
        // and that the duration marker is absent.
        let result = fmt_pace(30.0, ANCHOR_5H, 5000, FIVE_HOUR_SECS, true).unwrap();
        assert!(result.contains("🔥"), "expected 🔥 in {result:?}");
        assert!(result.contains(" (") && result.contains(')'),
            "expected parenthesised clock time in {result:?}");
        assert!(!result.contains("⏳"), "should not contain duration marker: {result:?}");
    }

    #[test]
    fn pace_shows_hours_and_minutes_when_early_by_more_than_one_hour() {
        // 60% used at 5000s → projected ≈ 216%; time_to_exhaust = 5000*100/60 ≈ 8333s
        // secs_early = 18000 - 8333 ≈ 9667s ≈ 161m → 2h 41m
        let result = fmt_pace(60.0, ANCHOR_5H, 5000, FIVE_HOUR_SECS, false).unwrap();
        assert!(result.contains("⏳-2h 41m"), "expected ⏳-2h 41m in {result:?}");
    }
}
