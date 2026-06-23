use std::io::{self, Read};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::TimeZone as _;
use serde_json::Value;
use unicode_width::UnicodeWidthChar;

const SEP: &str = "\x1b[2m │ \x1b[0m";
const FIVE_HOUR_SECS: u64 = 5 * 3600;
const SEVEN_DAY_SECS: u64 = 7 * 24 * 3600;
const PACE_MIN_PCT: f64 = 10.0;

// Each successive level applies one additional compression on top of the previous.
// Level 0 is the full layout; MAX is the most compact possible.
//   1-5:  bars 15 → 10 (one cell per step)
//   6:    effort "high" → "h"
//   7:    model "Sonnet 4.6" → "S 4.6"
//   8-12: bars 9 → 5 (one cell per step)
//   13:   drop cost
//   14:   7d bar → compact display (pace stays)
//   15:   5h bar → compact display (pace stays)
//   16:   ctx bar → compact display
//   17:   drop ctx size annotation
//   18:   drop model entirely
#[derive(Clone, Copy, PartialEq, Eq)]
struct CompressionLevel(u8);

impl CompressionLevel {
    const MAX: Self = Self(18);

    fn bar_width(self) -> usize {
        let n = usize::from(self.0);
        match self.0 {
            0..=5 => 15 - n,  // 15 down to 10
            6..=7 => 10,       // effort/model steps — bar stays at 10
            8..=12 => 17 - n, // 9 down to 5
            _ => 5,
        }
    }
    fn effort_compact(self) -> bool {
        self.0 >= 6
    }
    fn model_compact(self) -> bool {
        self.0 >= 7
    }
    fn show_cost(self) -> bool {
        self.0 < 13
    }
    fn compact_seven_d(self) -> bool {
        self.0 >= 14
    }
    fn compact_five_h(self) -> bool {
        self.0 >= 15
    }
    fn compact_ctx(self) -> bool {
        self.0 >= 16
    }
    fn show_ctx_size(self) -> bool {
        self.0 < 17
    }
    fn show_model(self) -> bool {
        self.0 < 18
    }
}

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

fn mini_bar(percent: u32, width: usize) -> String {
    let filled = ((percent as usize) * width / 100).min(width);
    let mut s = String::with_capacity(width * 3);
    for _ in 0..filled {
        s.push('▰');
    }
    for _ in filled..width {
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

// Returns the visible display width of a string, stripping ANSI CSI escape
// sequences and summing Unicode column widths of the remaining characters.
fn visible_len(s: &str) -> usize {
    let mut width = 0usize;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip CSI sequence: ESC [ <params> <final-byte>
            if chars.next_if_eq(&'[').is_some() {
                for inner in chars.by_ref() {
                    if inner.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else if c == '\u{FE0F}' {
            // Emoji presentation selector forces the preceding glyph to render
            // as a 2-wide emoji; add 1 here since the base char is already counted as 1.
            width += 1;
        } else {
            width += c.width().unwrap_or(0);
        }
    }
    width
}

fn terminal_columns() -> usize {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(usize::MAX)
}

// Abbreviates the first word of the model name to its initial: "Sonnet 4.6" → "S 4.6".
fn compact_model_name(model: &str) -> String {
    model.find(' ').map_or_else(
        || model.to_string(),
        |pos| {
            let initial = model.chars().next().unwrap_or('?');
            format!("{initial}{}", &model[pos..])
        },
    )
}

// In compact mode: "label:val%". In full mode: "label ▰▱▱ val%".
fn fmt_metric(label: &str, val: u32, compact: bool, bar_width: usize) -> String {
    if compact {
        format!("{label}:{val}%")
    } else {
        format!("{label} {} {val}%", mini_bar(val, bar_width))
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

struct StatusData {
    model: String,
    effort: Option<String>,
    ctx_used: Option<f64>,
    ctx_size: Option<u64>,
    five_h_used: Option<f64>,
    five_h_resets_at: Option<u64>,
    seven_d_used: Option<f64>,
    seven_d_resets_at: Option<u64>,
    cost: Option<f64>,
}

impl StatusData {
    fn from_json(v: &Value) -> Self {
        Self {
            model: v["model"]["display_name"].as_str().unwrap_or("").to_string(),
            effort: v["effort"]["level"].as_str().map(str::to_string),
            ctx_used: v["context_window"]["used_percentage"].as_f64(),
            ctx_size: v["context_window"]["context_window_size"].as_u64(),
            five_h_used: v["rate_limits"]["five_hour"]["used_percentage"].as_f64(),
            five_h_resets_at: v["rate_limits"]["five_hour"]["resets_at"].as_u64(),
            seven_d_used: v["rate_limits"]["seven_day"]["used_percentage"].as_f64(),
            seven_d_resets_at: v["rate_limits"]["seven_day"]["resets_at"].as_u64(),
            cost: v["cost"]["total_cost_usd"].as_f64(),
        }
    }
}

fn build_output(data: &StatusData, level: CompressionLevel, now: u64) -> String {
    let mut parts: Vec<String> = Vec::new();
    let bar_width = level.bar_width();

    if level.show_model() && !data.model.is_empty() {
        let model_str = if level.model_compact() {
            compact_model_name(&data.model)
        } else {
            data.model.clone()
        };
        let label = match &data.effort {
            Some(lvl) => {
                let lvl_display = if level.effort_compact() {
                    lvl.get(..1).unwrap_or(lvl.as_str())
                } else {
                    lvl.as_str()
                };
                format!("{model_str} ({lvl_display})")
            }
            None => model_str,
        };
        parts.push(format!("\x1b[1;35m{label}\x1b[0m"));
    }

    if let Some(ctx) = data.ctx_used {
        let val = pct_u32(ctx);
        let size = if level.show_ctx_size() {
            data.ctx_size
                .map_or(String::new(), |n| format!(" ({})", fmt_ctx_size(n)))
        } else {
            String::new()
        };
        parts.push(format!(
            "{}{}{size}\x1b[0m",
            color_by_used(val),
            fmt_metric("ctx", val, level.compact_ctx(), bar_width),
        ));
    }

    if let Some(used) = data.five_h_used {
        let val = pct_u32(used);
        let reset = data
            .five_h_resets_at
            .map(|r| format!(" ({})", fmt_clock_time(r)))
            .unwrap_or_default();
        let pace = data
            .five_h_resets_at
            .and_then(|r| fmt_pace(used, r, now, FIVE_HOUR_SECS, true))
            .map_or(String::new(), |p| format!(" {p}"));
        parts.push(format!(
            "{}{}{reset}\x1b[0m{pace}",
            color_by_used(val),
            fmt_metric("5h", val, level.compact_five_h(), bar_width),
        ));
    }

    if let Some(used) = data.seven_d_used {
        let val = pct_u32(used);
        let pace = data
            .seven_d_resets_at
            .and_then(|r| fmt_pace(used, r, now, SEVEN_DAY_SECS, false))
            .map_or(String::new(), |p| format!(" {p}"));
        parts.push(format!(
            "{}{}\x1b[0m{pace}",
            color_by_used(val),
            fmt_metric("7d", val, level.compact_seven_d(), bar_width),
        ));
    }

    if level.show_cost() {
        if let Some(c) = data.cost {
            if c > 0.0 {
                parts.push(format!("\x1b[2m$ {c:.2}\x1b[0m"));
            }
        }
    }

    parts.join(SEP)
}

fn main() {
    let mut input = String::new();
    let _ = io::stdin().read_to_string(&mut input);

    let v: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
    let data = StatusData::from_json(&v);
    let now = now_secs().unwrap_or(0);
    let cols = terminal_columns();

    let output = (0..=CompressionLevel::MAX.0)
        .find_map(|n| {
            let level = CompressionLevel(n);
            let s = build_output(&data, level, now);
            if visible_len(&s) <= cols || level == CompressionLevel::MAX {
                Some(s)
            } else {
                None
            }
        })
        .expect("loop always returns Some at MAX level");
    print!("{output}");
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
        assert_eq!(mini_bar(0, 10), "▱▱▱▱▱▱▱▱▱▱");
    }

    #[test]
    fn bar_is_all_filled_at_one_hundred_percent() {
        assert_eq!(mini_bar(100, 10), "▰▰▰▰▰▰▰▰▰▰");
    }

    #[test]
    fn bar_is_half_filled_at_fifty_percent() {
        assert_eq!(mini_bar(50, 10), "▰▰▰▰▰▱▱▱▱▱");
    }

    #[test]
    fn bar_respects_custom_width() {
        assert_eq!(mini_bar(100, 15), "▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰");
        assert_eq!(mini_bar(0, 5), "▱▱▱▱▱");
        assert_eq!(mini_bar(50, 6), "▰▰▰▱▱▱");
    }

    // --- visible_len ---

    #[test]
    fn visible_len_counts_plain_ascii() {
        assert_eq!(visible_len("hello"), 5);
    }

    #[test]
    fn visible_len_strips_ansi_escape_sequences() {
        assert_eq!(visible_len("\x1b[31mhello\x1b[0m"), 5);
    }

    #[test]
    fn visible_len_counts_wide_emoji_as_two_columns() {
        assert_eq!(visible_len("🔥"), 2);
        assert_eq!(visible_len("⏳"), 2);
        // ⚠️ is U+26A0 (width 1) + U+FE0F (emoji selector, adds 1) = 2
        assert_eq!(visible_len("⚠️"), 2);
    }

    #[test]
    fn visible_len_counts_sep_as_three_columns() {
        assert_eq!(visible_len(SEP), 3);
    }

    #[test]
    fn visible_len_handles_mixed_ansi_and_wide_chars() {
        // "\x1b[31m🔥\x1b[0m" → stripped = "🔥" → width 2
        assert_eq!(visible_len("\x1b[31m🔥\x1b[0m"), 2);
    }

    // --- CompressionLevel ---

    #[test]
    fn compression_level_zero_is_full_layout() {
        let l = CompressionLevel(0);
        assert_eq!(l.bar_width(), 15);
        assert!(!l.effort_compact());
        assert!(!l.model_compact());
        assert!(l.show_cost());
        assert!(!l.compact_seven_d());
        assert!(!l.compact_five_h());
        assert!(!l.compact_ctx());
        assert!(l.show_ctx_size());
        assert!(l.show_model());
    }

    #[test]
    fn compression_level_max_enables_all_compressions() {
        let l = CompressionLevel::MAX;
        assert_eq!(l.bar_width(), 5);
        assert!(l.effort_compact());
        assert!(l.model_compact());
        assert!(!l.show_cost());
        assert!(l.compact_seven_d());
        assert!(l.compact_five_h());
        assert!(l.compact_ctx());
        assert!(!l.show_ctx_size());
        assert!(!l.show_model());
    }

    #[test]
    fn compression_level_bar_width_decreases_through_levels() {
        assert_eq!(CompressionLevel(0).bar_width(), 15);
        assert_eq!(CompressionLevel(5).bar_width(), 10);
        assert_eq!(CompressionLevel(6).bar_width(), 10); // effort step, bar unchanged
        assert_eq!(CompressionLevel(7).bar_width(), 10); // model step, bar unchanged
        assert_eq!(CompressionLevel(8).bar_width(), 9);
        assert_eq!(CompressionLevel(12).bar_width(), 5);
        assert_eq!(CompressionLevel(18).bar_width(), 5);
    }

    #[test]
    fn compression_level_transitions_apply_one_step_at_a_time() {
        // level 12 has 5-wide bars but still shows cost
        let l12 = CompressionLevel(12);
        assert_eq!(l12.bar_width(), 5);
        assert!(l12.show_cost());
        assert!(!l12.compact_seven_d());
        // level 13 drops cost, bars stay at 5
        let l13 = CompressionLevel(13);
        assert!(!l13.show_cost());
        assert!(l13.show_model());
        assert!(!l13.compact_seven_d());
        // level 14 compacts 7d only
        let l14 = CompressionLevel(14);
        assert!(l14.compact_seven_d());
        assert!(!l14.compact_five_h());
        // level 17 drops ctx size but model still present
        let l17 = CompressionLevel(17);
        assert!(!l17.show_ctx_size());
        assert!(l17.show_model());
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
