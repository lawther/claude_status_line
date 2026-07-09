mod install;

use std::io::{self, Read};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::TimeZone as _;
use serde_json::Value;
use unicode_width::UnicodeWidthChar;

const SEP: &str = "\x1b[2m │ \x1b[0m";
const FIVE_HOUR_SECS: u64 = 5 * 3600;
const SEVEN_DAY_SECS: u64 = 7 * 24 * 3600;
const PACE_MIN_PCT: f64 = 10.0;

// Each variant applies one additional compression step on top of the previous.
// Full is the richest layout; DropCacheHitRate is the most compact possible.
// Bar variants shrink the progress bars in two phases (15→10, then 9→5).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CompressionLevel {
    Full,
    Bar14,
    Bar13,
    Bar12,
    Bar11,
    Bar10,
    EffortCompact,
    ModelCompact,
    Bar9,
    Bar8,
    Bar7,
    Bar6,
    Bar5,
    CompactCacheLabel,
    DropCost,
    CompactSevenDPace,
    CompactFiveHPace,
    CompactSevenD,
    CompactFiveH,
    CompactCtx,
    DropCtxSize,
    DropModel,
    DropCacheHitRate,
}

impl CompressionLevel {
    const MAX: Self = Self::DropCacheHitRate;

    const ALL: &'static [Self] = &[
        Self::Full,
        Self::Bar14,
        Self::Bar13,
        Self::Bar12,
        Self::Bar11,
        Self::Bar10,
        Self::EffortCompact,
        Self::ModelCompact,
        Self::Bar9,
        Self::Bar8,
        Self::Bar7,
        Self::Bar6,
        Self::Bar5,
        Self::CompactCacheLabel,
        Self::DropCost,
        Self::CompactSevenDPace,
        Self::CompactFiveHPace,
        Self::CompactSevenD,
        Self::CompactFiveH,
        Self::CompactCtx,
        Self::DropCtxSize,
        Self::DropModel,
        Self::DropCacheHitRate,
    ];

    fn bar_width(self) -> usize {
        match self {
            Self::Full => 15,
            Self::Bar14 => 14,
            Self::Bar13 => 13,
            Self::Bar12 => 12,
            Self::Bar11 => 11,
            Self::Bar10 | Self::EffortCompact | Self::ModelCompact => 10,
            Self::Bar9 => 9,
            Self::Bar8 => 8,
            Self::Bar7 => 7,
            Self::Bar6 => 6,
            Self::Bar5
            | Self::CompactCacheLabel
            | Self::DropCost
            | Self::CompactSevenDPace
            | Self::CompactFiveHPace
            | Self::CompactSevenD
            | Self::CompactFiveH
            | Self::CompactCtx
            | Self::DropCtxSize
            | Self::DropModel
            | Self::DropCacheHitRate => 5,
        }
    }

    fn effort_compact(self) -> bool {
        self >= Self::EffortCompact
    }
    fn model_compact(self) -> bool {
        self >= Self::ModelCompact
    }
    fn compact_cache_label(self) -> bool {
        self >= Self::CompactCacheLabel
    }
    fn show_cost(self) -> bool {
        self < Self::DropCost
    }
    fn compact_seven_d_pace(self) -> bool {
        self >= Self::CompactSevenDPace
    }
    fn compact_five_h_pace(self) -> bool {
        self >= Self::CompactFiveHPace
    }
    fn compact_seven_d(self) -> bool {
        self >= Self::CompactSevenD
    }
    fn compact_five_h(self) -> bool {
        self >= Self::CompactFiveH
    }
    fn compact_ctx(self) -> bool {
        self >= Self::CompactCtx
    }
    fn show_ctx_size(self) -> bool {
        self < Self::DropCtxSize
    }
    fn show_model(self) -> bool {
        self < Self::DropModel
    }
    fn show_cache_hit_rate(self) -> bool {
        self < Self::DropCacheHitRate
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
    let cols = std::env::var("COLUMNS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(usize::MAX);
    // Claude Code renders padding + content + padding inside a COLUMNS-wide box and
    // clips when the total >= COLUMNS (strict). With padding:2 on each side that is
    // 4 bytes of padding; subtracting 9 gives a comfortable margin clear of the clip boundary.
    cols.saturating_sub(9)
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

fn fmt_day_clock_time(unix_secs: u64) -> String {
    let Ok(secs_i64) = i64::try_from(unix_secs) else {
        return "--".to_string();
    };
    chrono::Local
        .timestamp_opt(secs_i64, 0)
        .single()
        .map_or_else(
            || "--".to_string(),
            |dt| {
                let day: String = dt.format("%a").to_string().chars().take(2).collect();
                format!("{day} {}", dt.format("%H:%M"))
            },
        )
}

#[derive(Clone, Copy)]
enum ExhaustionFormat {
    ClockTime,
    DayClockTime,
}

// Returns a pace indicator string once PACE_MIN_PCT of the window has elapsed or PACE_MIN_PCT
// of the quota has been used, whichever comes first. Green <90%, yellow 90-100%, red >100%.
// When projected >100%, shows exhaustion according to exhaustion_format.
#[allow(clippy::cast_precision_loss)]
fn fmt_pace(
    used_pct: f64,
    resets_at: u64,
    now: u64,
    window_secs: u64,
    exhaustion_format: ExhaustionFormat,
    compact: bool,
) -> Option<String> {
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
        ("\x1b[31m", "🔥") // red  — will exceed
    } else if pct >= 90 {
        ("\x1b[33m", "⚠️") // yellow — approaching
    } else {
        ("\x1b[32m", "✓") // green  — sustainable
    };

    let early_suffix = if pct > 100 {
        let time_to_exhaust_secs = (elapsed as f64) * 100.0 / used_pct;
        match exhaustion_format {
            ExhaustionFormat::ClockTime => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let exhaust_unix = window_start + time_to_exhaust_secs.round() as u64;
                format!(" ({})", fmt_clock_time(exhaust_unix))
            }
            ExhaustionFormat::DayClockTime => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let exhaust_unix = window_start + time_to_exhaust_secs.round() as u64;
                format!(" ({})", fmt_day_clock_time(exhaust_unix))
            }
        }
    } else {
        String::new()
    };

    if compact {
        Some(format!("{color}{symbol}{early_suffix}\x1b[0m"))
    } else {
        Some(format!("{color}{symbol} →{pct}%{early_suffix}\x1b[0m"))
    }
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
    cache_hit_rate: Option<f64>,
    cost: Option<f64>,
}

impl StatusData {
    #[allow(clippy::cast_precision_loss)]
    fn from_json(v: &Value) -> Self {
        let cache_hit_rate = v["context_window"]["current_usage"]
            .as_object()
            .and_then(|usage| {
                let read = usage
                    .get("cache_read_input_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let creation = usage
                    .get("cache_creation_input_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let denominator = read + creation;
                if denominator > 0 {
                    Some(read as f64 / denominator as f64 * 100.0)
                } else {
                    None
                }
            });
        Self {
            model: v["model"]["display_name"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            effort: v["effort"]["level"].as_str().map(str::to_string),
            ctx_used: v["context_window"]["used_percentage"].as_f64(),
            ctx_size: v["context_window"]["context_window_size"].as_u64(),
            five_h_used: v["rate_limits"]["five_hour"]["used_percentage"].as_f64(),
            five_h_resets_at: v["rate_limits"]["five_hour"]["resets_at"].as_u64(),
            seven_d_used: v["rate_limits"]["seven_day"]["used_percentage"].as_f64(),
            seven_d_resets_at: v["rate_limits"]["seven_day"]["resets_at"].as_u64(),
            cache_hit_rate,
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
            .and_then(|r| {
                fmt_pace(
                    used,
                    r,
                    now,
                    FIVE_HOUR_SECS,
                    ExhaustionFormat::ClockTime,
                    level.compact_five_h_pace(),
                )
            })
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
            .and_then(|r| {
                fmt_pace(
                    used,
                    r,
                    now,
                    SEVEN_DAY_SECS,
                    ExhaustionFormat::DayClockTime,
                    level.compact_seven_d_pace(),
                )
            })
            .map_or(String::new(), |p| format!(" {p}"));
        parts.push(format!(
            "{}{}\x1b[0m{pace}",
            color_by_used(val),
            fmt_metric("7d", val, level.compact_seven_d(), bar_width),
        ));
    }

    if level.show_cache_hit_rate() {
        if let Some(rate) = data.cache_hit_rate {
            let prefix = if level.compact_cache_label() {
                ""
            } else {
                "cache "
            };
            parts.push(format!("\x1b[2m{prefix}♻️ {}%\x1b[0m", pct_u32(rate)));
        }
    }

    if level.show_cost() {
        if let Some(c) = data.cost {
            if c > 0.0 {
                parts.push(format!("\x1b[2m${c:.2}\x1b[0m"));
            }
        }
    }

    parts.join(SEP)
}

fn main() {
    if std::env::args().any(|a| a == "--help" || a == "-h") {
        println!("claude_status_line — Claude Code status line renderer");
        println!();
        println!("\x1b[1mUSAGE\x1b[0m");
        println!("  claude_status_line [--install [-q|--quiet] [--link]]");
        println!("  echo '{{...}}' | claude_status_line");
        println!();
        println!("\x1b[1mOPTIONS\x1b[0m");
        println!("  \x1b[1m--install\x1b[0m         Copy this binary to ~/.claude/statusline and");
        println!("                    configure it in ~/.claude/settings.json");
        println!(
            "  \x1b[1m--link\x1b[0m            Configure settings.json directly with the current"
        );
        println!("                    binary path instead of copying it");
        println!("  \x1b[1m-q, --quiet\x1b[0m       Suppress output from --install");
        println!("  \x1b[1m-h, --help\x1b[0m        Show this help message");
        println!();
        println!("\x1b[1mNORMAL OPERATION\x1b[0m");
        println!("  Reads a Claude Code status JSON object from stdin and writes");
        println!("  a formatted, colour-coded status line to stdout. Compresses");
        println!("  output automatically to fit the terminal width.");
        return;
    }

    if std::env::args().any(|a| a == "--install") {
        let quiet = std::env::args().any(|a| a == "--quiet" || a == "-q");
        let link = std::env::args().any(|a| a == "--link");
        if let Err(e) = install::run(quiet, link) {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return;
    }

    let mut input = String::new();
    let _ = io::stdin().read_to_string(&mut input);

    let v: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
    let data = StatusData::from_json(&v);
    let now = now_secs().unwrap_or(0);
    let cols = terminal_columns();

    let output = CompressionLevel::ALL
        .iter()
        .find_map(|&level| {
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
    fn compression_level_full_is_full_layout() {
        let l = CompressionLevel::Full;
        assert_eq!(l.bar_width(), 15);
        assert!(!l.effort_compact());
        assert!(!l.model_compact());
        assert!(!l.compact_cache_label());
        assert!(l.show_cost());
        assert!(!l.compact_seven_d_pace());
        assert!(!l.compact_five_h_pace());
        assert!(!l.compact_seven_d());
        assert!(!l.compact_five_h());
        assert!(!l.compact_ctx());
        assert!(l.show_ctx_size());
        assert!(l.show_model());
        assert!(l.show_cache_hit_rate());
    }

    #[test]
    fn compression_level_max_enables_all_compressions() {
        let l = CompressionLevel::MAX;
        assert_eq!(l.bar_width(), 5);
        assert!(l.effort_compact());
        assert!(l.model_compact());
        assert!(l.compact_cache_label());
        assert!(!l.show_cost());
        assert!(l.compact_seven_d_pace());
        assert!(l.compact_five_h_pace());
        assert!(l.compact_seven_d());
        assert!(l.compact_five_h());
        assert!(l.compact_ctx());
        assert!(!l.show_ctx_size());
        assert!(!l.show_model());
        assert!(!l.show_cache_hit_rate());
    }

    #[test]
    fn compression_level_bar_width_decreases_through_levels() {
        assert_eq!(CompressionLevel::Full.bar_width(), 15);
        assert_eq!(CompressionLevel::Bar10.bar_width(), 10);
        assert_eq!(CompressionLevel::EffortCompact.bar_width(), 10); // effort step, bar unchanged
        assert_eq!(CompressionLevel::ModelCompact.bar_width(), 10); // model step, bar unchanged
        assert_eq!(CompressionLevel::Bar9.bar_width(), 9);
        assert_eq!(CompressionLevel::Bar5.bar_width(), 5);
        assert_eq!(CompressionLevel::MAX.bar_width(), 5);
    }

    #[test]
    fn compression_level_transitions_apply_one_step_at_a_time() {
        // Bar5 has 5-wide bars, full cache label, cost still shown
        let bar5 = CompressionLevel::Bar5;
        assert_eq!(bar5.bar_width(), 5);
        assert!(!bar5.compact_cache_label());
        assert!(bar5.show_cost());
        assert!(!bar5.compact_seven_d_pace());
        // CompactCacheLabel compacts cache label only
        let ccl = CompressionLevel::CompactCacheLabel;
        assert!(ccl.compact_cache_label());
        assert!(ccl.show_cost());
        assert!(!ccl.compact_seven_d_pace());
        // DropCost drops cost; pace still full
        let dc = CompressionLevel::DropCost;
        assert!(!dc.show_cost());
        assert!(!dc.compact_seven_d_pace());
        // CompactSevenDPace compacts 7d pace only
        let c7p = CompressionLevel::CompactSevenDPace;
        assert!(c7p.compact_seven_d_pace());
        assert!(!c7p.compact_five_h_pace());
        // CompactFiveHPace compacts 5h pace; bars still full
        let c5p = CompressionLevel::CompactFiveHPace;
        assert!(c5p.compact_five_h_pace());
        assert!(!c5p.compact_seven_d());
        // CompactSevenD compacts 7d bar only
        let c7 = CompressionLevel::CompactSevenD;
        assert!(c7.compact_seven_d());
        assert!(!c7.compact_five_h());
        // DropCtxSize drops ctx size but model still present
        let dcs = CompressionLevel::DropCtxSize;
        assert!(!dcs.show_ctx_size());
        assert!(dcs.show_model());
        assert!(dcs.show_cache_hit_rate());
        // DropModel drops model but cache hit rate persists
        let dm = CompressionLevel::DropModel;
        assert!(!dm.show_model());
        assert!(dm.show_cache_hit_rate());
        // DropCacheHitRate drops cache hit rate
        assert!(!CompressionLevel::DropCacheHitRate.show_cache_hit_rate());
    }

    // --- cache_hit_rate ---

    #[test]
    fn cache_hit_rate_computed_from_read_and_creation_tokens() {
        let json = serde_json::json!({
            "model": {"display_name": "Sonnet 4.6"},
            "context_window": {
                "total_input_tokens": 1000,
                "current_usage": {
                    "cache_read_input_tokens": 750,
                    "cache_creation_input_tokens": 250
                }
            }
        });
        let data = StatusData::from_json(&json);
        let rate = data.cache_hit_rate.expect("should have cache hit rate");
        assert!((rate - 75.0).abs() < 0.01, "expected 75%, got {rate}");
    }

    #[test]
    fn cache_hit_rate_absent_when_tokens_missing() {
        let json = serde_json::json!({"model": {"display_name": "Sonnet 4.6"}});
        let data = StatusData::from_json(&json);
        assert!(data.cache_hit_rate.is_none());
    }

    #[test]
    fn cache_hit_rate_absent_when_total_tokens_zero() {
        let json = serde_json::json!({
            "model": {"display_name": "Sonnet 4.6"},
            "context_window": {
                "total_input_tokens": 0,
                "current_usage": {"cache_read_input_tokens": 0}
            }
        });
        let data = StatusData::from_json(&json);
        assert!(data.cache_hit_rate.is_none());
    }

    #[test]
    fn cache_hit_rate_appears_in_output_between_seven_d_and_cost() {
        let json = serde_json::json!({
            "model": {"display_name": "Sonnet 4.6"},
            "context_window": {
                "total_input_tokens": 1000,
                "current_usage": {"cache_read_input_tokens": 800},
                "used_percentage": 20,
                "context_window_size": 200000
            },
            "rate_limits": {
                "seven_day": {"used_percentage": 10, "resets_at": 9999999999_u64}
            },
            "cost": {"total_cost_usd": 1.23}
        });
        let data = StatusData::from_json(&json);
        let output = build_output(&data, CompressionLevel::Full, 0);
        let seven_d_pos = output.find("7d").expect("7d not found");
        let cache_pos = output.find('\u{267B}').expect("♻ not found");
        let cost_pos = output.find('$').expect("$ not found");
        assert!(
            seven_d_pos < cache_pos && cache_pos < cost_pos,
            "expected 7d … ♻️ … $, but positions were: 7d={seven_d_pos} cache={cache_pos} $={cost_pos}"
        );
    }

    #[test]
    fn cache_hit_rate_shows_full_label_before_cache_label_compresses() {
        let json = serde_json::json!({
            "model": {"display_name": "Sonnet 4.6"},
            "context_window": {
                "total_input_tokens": 1000,
                "current_usage": {"cache_read_input_tokens": 800}
            }
        });
        let data = StatusData::from_json(&json);
        let output = build_output(&data, CompressionLevel::Bar5, 0);
        assert!(
            output.contains("cache \u{267B}"),
            "expected 'cache ♻' at Bar5: {output:?}"
        );
    }

    #[test]
    fn cache_hit_rate_drops_label_at_level_13() {
        let json = serde_json::json!({
            "model": {"display_name": "Sonnet 4.6"},
            "context_window": {
                "total_input_tokens": 1000,
                "current_usage": {"cache_read_input_tokens": 800}
            }
        });
        let data = StatusData::from_json(&json);
        let output = build_output(&data, CompressionLevel::CompactCacheLabel, 0);
        assert!(
            output.contains('\u{267B}'),
            "expected ♻️ at CompactCacheLabel"
        );
        assert!(
            !output.contains("cache \u{267B}"),
            "should not show 'cache ♻' at CompactCacheLabel: {output:?}"
        );
    }

    #[test]
    fn cache_hit_rate_hidden_at_max_compression() {
        let json = serde_json::json!({
            "model": {"display_name": "Sonnet 4.6"},
            "context_window": {
                "total_input_tokens": 1000,
                "current_usage": {"cache_read_input_tokens": 800},
                "used_percentage": 20,
                "context_window_size": 200000
            }
        });
        let data = StatusData::from_json(&json);
        let output = build_output(&data, CompressionLevel::MAX, 0);
        assert!(
            !output.contains('\u{267B}'),
            "♻️ should not appear at MAX compression"
        );
    }

    // --- fmt_pace: threshold triggering ---

    #[test]
    fn pace_suppressed_when_both_time_and_usage_below_threshold() {
        // 8% elapsed, 9% used — both below the 10% threshold
        let elapsed = elapsed_pct(FIVE_HOUR_SECS, 8.0);
        assert!(fmt_pace(
            9.0,
            ANCHOR_5H,
            elapsed,
            FIVE_HOUR_SECS,
            ExhaustionFormat::ClockTime,
            false,
        )
        .is_none());
    }

    #[test]
    fn pace_shown_when_usage_meets_threshold_before_time_does() {
        // 8% elapsed, 11% used — usage crosses 10% while time has not
        let elapsed = elapsed_pct(FIVE_HOUR_SECS, 8.0);
        assert!(fmt_pace(
            11.0,
            ANCHOR_5H,
            elapsed,
            FIVE_HOUR_SECS,
            ExhaustionFormat::ClockTime,
            false,
        )
        .is_some());
    }

    #[test]
    fn pace_shown_when_time_meets_threshold_before_usage_does() {
        // 11% elapsed, 9% used — time crosses 10% while usage has not
        let elapsed = elapsed_pct(FIVE_HOUR_SECS, 11.0);
        assert!(fmt_pace(
            9.0,
            ANCHOR_5H,
            elapsed,
            FIVE_HOUR_SECS,
            ExhaustionFormat::ClockTime,
            false,
        )
        .is_some());
    }

    #[test]
    fn pace_suppressed_when_window_is_complete() {
        // elapsed = window — nothing left to project
        assert!(fmt_pace(
            90.0,
            ANCHOR_5H,
            FIVE_HOUR_SECS,
            FIVE_HOUR_SECS,
            ExhaustionFormat::ClockTime,
            false,
        )
        .is_none());
    }

    #[test]
    fn pace_thresholds_apply_independently_to_seven_day_window() {
        // 8% elapsed, 9% used on 7d window — suppressed
        let elapsed = elapsed_pct(SEVEN_DAY_SECS, 8.0);
        assert!(fmt_pace(
            9.0,
            ANCHOR_7D,
            elapsed,
            SEVEN_DAY_SECS,
            ExhaustionFormat::ClockTime,
            false,
        )
        .is_none());
        // 11% elapsed — shown
        let elapsed = elapsed_pct(SEVEN_DAY_SECS, 11.0);
        assert!(fmt_pace(
            9.0,
            ANCHOR_7D,
            elapsed,
            SEVEN_DAY_SECS,
            ExhaustionFormat::ClockTime,
            false,
        )
        .is_some());
    }

    // --- fmt_pace: projected rate colouring ---
    // elapsed = 5000s (~28% of 5h) for all colour tests, keeping well past the threshold.

    #[test]
    fn pace_green_when_projected_under_ninety_percent() {
        // 20% used at 28% elapsed → projected ≈ 72%
        let result = fmt_pace(
            20.0,
            ANCHOR_5H,
            5000,
            FIVE_HOUR_SECS,
            ExhaustionFormat::ClockTime,
            false,
        )
        .unwrap();
        assert!(result.contains('✓'), "expected ✓ in {result:?}");
    }

    #[test]
    fn pace_yellow_when_projected_between_ninety_and_one_hundred_percent() {
        // 26% used at 28% elapsed → projected ≈ 94%
        let result = fmt_pace(
            26.0,
            ANCHOR_5H,
            5000,
            FIVE_HOUR_SECS,
            ExhaustionFormat::ClockTime,
            false,
        )
        .unwrap();
        assert!(result.contains("⚠️"), "expected ⚠️ in {result:?}");
    }

    #[test]
    fn pace_red_when_projected_over_one_hundred_percent() {
        // 30% used at 28% elapsed → projected ≈ 108%
        let result = fmt_pace(
            30.0,
            ANCHOR_5H,
            5000,
            FIVE_HOUR_SECS,
            ExhaustionFormat::ClockTime,
            false,
        )
        .unwrap();
        assert!(result.contains("🔥"), "expected 🔥 in {result:?}");
    }

    // --- fmt_pace: exhaustion marker when projected > 100% ---

    #[test]
    fn pace_shows_clock_time_when_exceeding_with_clock_time_format() {
        // Exact time is timezone-dependent; verify the shape "(HH:MM)" is present
        // and that the duration marker is absent.
        let result = fmt_pace(
            30.0,
            ANCHOR_5H,
            5000,
            FIVE_HOUR_SECS,
            ExhaustionFormat::ClockTime,
            false,
        )
        .unwrap();
        assert!(result.contains("🔥"), "expected 🔥 in {result:?}");
        assert!(
            result.contains(" (") && result.contains(')'),
            "expected parenthesised clock time in {result:?}"
        );
        assert!(
            !result.contains("⏳"),
            "should not contain duration marker: {result:?}"
        );
    }

    #[test]
    fn pace_shows_day_clock_time_when_exceeding_with_day_clock_time_format() {
        let result = fmt_pace(
            30.0,
            ANCHOR_5H,
            5000,
            FIVE_HOUR_SECS,
            ExhaustionFormat::DayClockTime,
            false,
        )
        .unwrap();
        assert!(result.contains("🔥"), "expected 🔥 in {result:?}");
        assert!(
            result.contains(" (") && result.contains(')'),
            "expected parenthesised time in {result:?}"
        );
        assert!(
            !result.contains("⏳"),
            "should not contain duration marker: {result:?}"
        );
    }

    #[test]
    fn pace_compact_drops_percentage_and_arrow_but_keeps_symbol_and_time() {
        // 30% used at 5000s → projected ≈ 108%, will show exhaustion time
        let full = fmt_pace(
            30.0,
            ANCHOR_5H,
            5000,
            FIVE_HOUR_SECS,
            ExhaustionFormat::ClockTime,
            false,
        )
        .unwrap();
        let compact = fmt_pace(
            30.0,
            ANCHOR_5H,
            5000,
            FIVE_HOUR_SECS,
            ExhaustionFormat::ClockTime,
            true,
        )
        .unwrap();
        assert!(full.contains("→"), "full should contain →: {full:?}");
        assert!(
            !compact.contains("→"),
            "compact should not contain →: {compact:?}"
        );
        assert!(
            compact.contains("🔥"),
            "compact should still show 🔥: {compact:?}"
        );
        assert!(
            compact.contains(" (") && compact.contains(')'),
            "compact should still show time: {compact:?}"
        );
    }

    #[test]
    fn pace_compact_without_exhaustion_shows_only_symbol() {
        // 20% used at 28% elapsed → projected ≈ 72%, green, no exhaustion time
        let compact = fmt_pace(
            20.0,
            ANCHOR_5H,
            5000,
            FIVE_HOUR_SECS,
            ExhaustionFormat::ClockTime,
            true,
        )
        .unwrap();
        assert!(
            !compact.contains("→"),
            "compact should not contain →: {compact:?}"
        );
        assert!(
            compact.contains('✓'),
            "compact should still show ✓: {compact:?}"
        );
    }
}
