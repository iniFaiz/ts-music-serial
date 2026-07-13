//! Native smart-playlist rule evaluation.

use rusqlite::Connection;
use serde_json::Value;

use crate::MusicTrack;

use super::{now_ms, row_to_track, TRACK_COLS_T};

// ---- Smart-playlist evaluation (native port of smartPlaylists.js) -----------

// A track plus the extra per-track signals smart rules can test.
struct SmartTrack {
    track: MusicTrack,
    play_count: i64,
    last_played: i64,
    favorite: bool,
}

pub(super) fn smart_eval(
    conn: &Connection,
    rules: &Value,
    sort_by: &str,
    sort_order: &str,
    limit: i64,
) -> Result<Vec<MusicTrack>, String> {
    // Load every track with its stats + favorite flag in one pass.
    let sql = format!(
        "SELECT {TRACK_COLS_T}, COALESCE(s.play_count, 0), COALESCE(s.last_played, 0),
                CASE WHEN f.path IS NULL THEN 0 ELSE 1 END
         FROM tracks t
         LEFT JOIN stats s ON s.path = t.path
         LEFT JOIN favorites f ON f.path = t.path"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(SmartTrack {
                track: row_to_track(r)?,
                play_count: r.get(14)?,
                last_played: r.get(15)?,
                favorite: r.get::<_, i64>(16)? != 0,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut items: Vec<SmartTrack> = Vec::new();
    for r in rows {
        items.push(r.map_err(|e| e.to_string())?);
    }

    let match_all = rules
        .get("match")
        .and_then(|v| v.as_str())
        .map(|s| s != "any")
        .unwrap_or(true);
    let empty = Vec::new();
    let conditions: Vec<&Value> = rules
        .get("conditions")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty)
        .iter()
        .filter(|c| c.get("field").is_some() && c.get("op").is_some())
        .collect();

    let now = now_ms();
    let mut filtered: Vec<SmartTrack> = items
        .into_iter()
        .filter(|it| {
            if conditions.is_empty() {
                return true;
            }
            if match_all {
                conditions.iter().all(|c| match_condition(c, it, now))
            } else {
                conditions.iter().any(|c| match_condition(c, it, now))
            }
        })
        .collect();

    sort_smart(&mut filtered, sort_by, sort_order);

    let mut out: Vec<MusicTrack> = filtered.into_iter().map(|it| it.track).collect();
    if limit > 0 && (out.len() as i64) > limit {
        out.truncate(limit as usize);
    }
    Ok(out)
}

// Comparable numeric value for the number/date sort + comparison ops.
fn field_number(field: &str, it: &SmartTrack) -> f64 {
    match field {
        "year" => it.track.year.unwrap_or(0) as f64,
        "duration" => it.track.duration_secs as f64,
        "playCount" => it.play_count as f64,
        "lastPlayed" => it.last_played as f64,
        "dateAdded" => (it.track.date_added as f64) * 1000.0, // seconds → ms epoch
        _ => 0.0,
    }
}

fn field_text<'a>(field: &str, it: &'a SmartTrack) -> &'a str {
    match field {
        "title" => &it.track.title,
        "artist" => &it.track.artist,
        "album" => &it.track.album,
        "genre" => it.track.genre.as_deref().unwrap_or(""),
        _ => "",
    }
}

fn match_condition(cond: &Value, it: &SmartTrack, now: i64) -> bool {
    let field = cond.get("field").and_then(|v| v.as_str()).unwrap_or("");
    let op = cond.get("op").and_then(|v| v.as_str()).unwrap_or("");
    let val = cond.get("value");

    match field {
        // Text fields.
        "title" | "artist" | "album" | "genre" => {
            let a = field_text(field, it).to_lowercase();
            let b = val.and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
            match op {
                "contains" => a.contains(&b),
                "notContains" => !a.contains(&b),
                "is" => a == b,
                "isNot" => a != b,
                "startsWith" => a.starts_with(&b),
                "endsWith" => a.ends_with(&b),
                _ => true,
            }
        }
        // Numeric fields (year, duration, playCount).
        "year" | "duration" | "playCount" => {
            let a = field_number(field, it);
            let b = val.map(json_to_f64).unwrap_or(0.0);
            match op {
                "is" => a == b,
                "isNot" => a != b,
                "gt" => a > b,
                "lt" => a < b,
                "gte" => a >= b,
                "lte" => a <= b,
                _ => true,
            }
        }
        // Date fields (lastPlayed, dateAdded) — value is a day count.
        "lastPlayed" | "dateAdded" => {
            let a = field_number(field, it); // ms epoch, 0 = never
            let days = val.map(json_to_f64).unwrap_or(0.0);
            let cutoff = now as f64 - days * 86_400_000.0;
            match op {
                "inLast" => a > 0.0 && a >= cutoff,
                "notInLast" => a == 0.0 || a < cutoff,
                "played" => a > 0.0,
                "never" => a == 0.0,
                _ => true,
            }
        }
        // Boolean.
        "favorite" => {
            if op == "isFalse" {
                !it.favorite
            } else {
                it.favorite
            }
        }
        _ => true,
    }
}

fn json_to_f64(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        Value::String(s) => s.trim().parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn sort_smart(items: &mut [SmartTrack], sort_by: &str, sort_order: &str) {
    if sort_by.is_empty() || sort_by == "none" {
        return;
    }
    if sort_by == "random" {
        // Fisher–Yates using a cheap xorshift seeded from the clock.
        let mut seed = now_ms() as u64 | 1;
        let mut rng = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for i in (1..items.len()).rev() {
            let j = (rng() % (i as u64 + 1)) as usize;
            items.swap(i, j);
        }
        return;
    }
    let desc = sort_order.eq_ignore_ascii_case("desc");
    let is_text = matches!(sort_by, "title" | "artist" | "album" | "genre");
    items.sort_by(|a, b| {
        let ord = if is_text {
            field_text(sort_by, a)
                .to_lowercase()
                .cmp(&field_text(sort_by, b).to_lowercase())
        } else {
            field_number(sort_by, a)
                .partial_cmp(&field_number(sort_by, b))
                .unwrap_or(std::cmp::Ordering::Equal)
        };
        if desc {
            ord.reverse()
        } else {
            ord
        }
    });
}

// Query cover art bytes for a track path from SQLite
