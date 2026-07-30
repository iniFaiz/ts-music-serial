//! SQL compiler for native smart-playlist rules.

use rusqlite::types::Value as SqlValue;
use rusqlite::{params_from_iter, Connection};
use serde_json::Value;

use crate::MusicTrack;

use super::{now_ms, random_u64, row_to_track, TRACK_COLS_T};

/// The dynamic part of a smart-playlist query. Every SQL identifier and
/// operator is selected from an allowlist below; user values only ever travel
/// through SQLite bind parameters.
#[derive(Clone, Debug)]
struct CompiledRules {
    where_sql: String,
    params: Vec<SqlValue>,
}

pub(super) fn smart_eval(
    conn: &Connection,
    rules: &Value,
    sort_by: &str,
    sort_order: &str,
    limit: i64,
) -> Result<Vec<MusicTrack>, String> {
    let compiled = compile_rules(rules, now_ms());
    if sort_by == "random" {
        return smart_eval_random(conn, &compiled, limit);
    }
    let mut sql = format!(
        "SELECT {TRACK_COLS_T}
         FROM tracks t
         LEFT JOIN stats s ON s.track_id = t.id
         LEFT JOIN favorites f ON f.track_id = t.id
         WHERE {}",
        compiled.where_sql
    );

    if let Some(order_by) = compile_order_by(sort_by, sort_order) {
        sql.push_str(" ORDER BY ");
        sql.push_str(order_by);
    }

    let mut params = compiled.params;
    if limit > 0 {
        sql.push_str(" LIMIT ?");
        params.push(SqlValue::Integer(limit));
    }

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params_from_iter(params.iter()), row_to_track)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

fn smart_random_segment(
    conn: &Connection,
    compiled: &CompiledRules,
    comparison: &str,
    start: i64,
    limit: Option<i64>,
) -> Result<Vec<MusicTrack>, String> {
    let mut sql = format!(
        "SELECT {TRACK_COLS_T}
         FROM tracks t
         LEFT JOIN stats s ON s.track_id = t.id
         LEFT JOIN favorites f ON f.track_id = t.id
         WHERE ({}) AND t.id {comparison} ?
         ORDER BY t.id",
        compiled.where_sql
    );
    let mut params = compiled.params.clone();
    params.push(SqlValue::Integer(start));
    if let Some(limit) = limit {
        sql.push_str(" LIMIT ?");
        params.push(SqlValue::Integer(limit));
    }
    let mut statement = conn.prepare(&sql).map_err(|error| error.to_string())?;
    let tracks = statement
        .query_map(params_from_iter(params.iter()), row_to_track)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(tracks)
}

fn smart_eval_random(
    conn: &Connection,
    compiled: &CompiledRules,
    limit: i64,
) -> Result<Vec<MusicTrack>, String> {
    let max_sql = format!(
        "SELECT COALESCE(MAX(t.id), 0)
         FROM tracks t
         LEFT JOIN stats s ON s.track_id = t.id
         LEFT JOIN favorites f ON f.track_id = t.id
         WHERE {}",
        compiled.where_sql
    );
    let max_id: i64 = conn
        .query_row(&max_sql, params_from_iter(compiled.params.iter()), |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())?;
    if max_id <= 0 {
        return Ok(Vec::new());
    }
    let start = (random_u64() % max_id as u64) as i64 + 1;
    let bounded = (limit > 0).then_some(limit);
    let mut tracks = smart_random_segment(conn, compiled, ">=", start, bounded)?;
    let remaining = if limit > 0 {
        limit.saturating_sub(tracks.len() as i64)
    } else {
        0
    };
    if limit == 0 || remaining > 0 {
        tracks.extend(smart_random_segment(
            conn,
            compiled,
            "<",
            start,
            (limit > 0).then_some(remaining),
        )?);
    }
    Ok(tracks)
}

/// Count matches without materializing every track. A positive playlist limit
/// is applied inside the subquery so the number shown in the UI is identical to
/// `smart_eval(...).len()`.
pub(super) fn smart_count(conn: &Connection, rules: &Value, limit: i64) -> Result<i64, String> {
    let compiled = compile_rules(rules, now_ms());
    let mut sql = format!(
        "SELECT COUNT(*) FROM (
           SELECT 1
           FROM tracks t
           LEFT JOIN stats s ON s.track_id = t.id
           LEFT JOIN favorites f ON f.track_id = t.id
           WHERE {}",
        compiled.where_sql
    );
    let mut params = compiled.params;
    if limit > 0 {
        sql.push_str(" LIMIT ?");
        params.push(SqlValue::Integer(limit));
    }
    sql.push(')');

    conn.query_row(&sql, params_from_iter(params.iter()), |row| row.get(0))
        .map_err(|e| e.to_string())
}

fn compile_rules(rules: &Value, now: i64) -> CompiledRules {
    let empty = Vec::new();
    let conditions: Vec<&Value> = rules
        .get("conditions")
        .and_then(Value::as_array)
        .unwrap_or(&empty)
        .iter()
        .filter(|condition| condition.get("field").is_some() && condition.get("op").is_some())
        .collect();

    if conditions.is_empty() {
        return CompiledRules {
            where_sql: "1 = 1".to_string(),
            params: Vec::new(),
        };
    }

    let joiner = if rules.get("match").and_then(Value::as_str) == Some("any") {
        " OR "
    } else {
        " AND "
    };
    let mut params = Vec::new();
    let clauses = conditions
        .into_iter()
        .map(|condition| compile_condition(condition, now, &mut params))
        .collect::<Vec<_>>();

    CompiledRules {
        where_sql: format!("({})", clauses.join(joiner)),
        params,
    }
}

fn compile_condition(condition: &Value, now: i64, params: &mut Vec<SqlValue>) -> String {
    let field = condition.get("field").and_then(Value::as_str).unwrap_or("");
    let op = condition.get("op").and_then(Value::as_str).unwrap_or("");
    let value = condition.get("value");

    if let Some(column) = text_column(field) {
        let raw = value.and_then(Value::as_str).unwrap_or("");
        return match op {
            "contains" => bind_like(column, format!("%{}%", escape_like(raw)), params, false),
            "notContains" => bind_like(column, format!("%{}%", escape_like(raw)), params, true),
            "startsWith" => bind_like(column, format!("{}%", escape_like(raw)), params, false),
            "endsWith" => bind_like(column, format!("%{}", escape_like(raw)), params, false),
            "is" | "isNot" => {
                params.push(SqlValue::Text(raw.to_string()));
                let operator = if op == "isNot" { "<>" } else { "=" };
                format!("COALESCE({column}, '') {operator} ? COLLATE NOCASE")
            }
            _ => "1 = 1".to_string(),
        };
    }

    if let Some(column) = numeric_column(field) {
        let operator = match op {
            "is" => "=",
            "isNot" => "<>",
            "gt" => ">",
            "lt" => "<",
            "gte" => ">=",
            "lte" => "<=",
            _ => return "1 = 1".to_string(),
        };
        params.push(SqlValue::Real(value.map(json_to_f64).unwrap_or(0.0)));
        return format!("{column} {operator} ?");
    }

    if let Some(column) = date_column(field) {
        return match op {
            "inLast" | "notInLast" => {
                let days = value.map(json_to_f64).unwrap_or(0.0);
                let cutoff = now as f64 - days * 86_400_000.0;
                params.push(SqlValue::Real(cutoff));
                if op == "inLast" {
                    format!("({column} > 0 AND {column} >= ?)")
                } else {
                    format!("({column} = 0 OR {column} < ?)")
                }
            }
            "played" => format!("{column} > 0"),
            "never" => format!("{column} = 0"),
            _ => "1 = 1".to_string(),
        };
    }

    if field == "favorite" {
        return if op == "isFalse" {
            "f.track_id IS NULL".to_string()
        } else {
            // This intentionally mirrors the previous evaluator: every boolean
            // operator except isFalse means "is true".
            "f.track_id IS NOT NULL".to_string()
        };
    }

    // Unknown fields/operators were historically no-op conditions. Keeping that
    // behaviour makes old or forward-versioned playlist JSON safe to open.
    "1 = 1".to_string()
}

fn text_column(field: &str) -> Option<&'static str> {
    match field {
        "title" => Some("t.title"),
        "artist" => Some("t.artist"),
        "album" => Some("t.album"),
        "genre" => Some("t.genre"),
        _ => None,
    }
}

fn numeric_column(field: &str) -> Option<&'static str> {
    match field {
        "year" => Some("COALESCE(t.year, 0)"),
        "duration" => Some("t.duration_secs"),
        "playCount" => Some("COALESCE(s.play_count, 0)"),
        _ => None,
    }
}

fn date_column(field: &str) -> Option<&'static str> {
    match field {
        "lastPlayed" => Some("COALESCE(s.last_played, 0)"),
        // Track dates are stored as seconds; smart rules compare milliseconds.
        "dateAdded" => Some("(t.first_seen_at * 1000.0)"),
        _ => None,
    }
}

fn bind_like(column: &str, pattern: String, params: &mut Vec<SqlValue>, negated: bool) -> String {
    params.push(SqlValue::Text(pattern));
    let not = if negated { "NOT " } else { "" };
    format!("{not}(COALESCE({column}, '') LIKE ? ESCAPE '^' COLLATE NOCASE)")
}

// SQLite LIKE treats '%' and '_' as wildcards. Smart-playlist text rules treat
// them literally, so escape those characters (and the escape character itself).
fn escape_like(value: &str) -> String {
    value
        .replace('^', "^^")
        .replace('%', "^%")
        .replace('_', "^_")
}

fn json_to_f64(value: &Value) -> f64 {
    match value {
        Value::Number(number) => number.as_f64().unwrap_or(0.0),
        Value::String(text) => text.trim().parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn compile_order_by(sort_by: &str, sort_order: &str) -> Option<&'static str> {
    let desc = sort_order.eq_ignore_ascii_case("desc");
    match (sort_by, desc) {
        ("random", _) => None,
        ("title", false) => Some("t.title COLLATE NOCASE ASC, t.id ASC"),
        ("title", true) => Some("t.title COLLATE NOCASE DESC, t.id ASC"),
        ("artist", false) => Some("t.artist COLLATE NOCASE ASC, t.id ASC"),
        ("artist", true) => Some("t.artist COLLATE NOCASE DESC, t.id ASC"),
        ("album", false) => Some("t.album COLLATE NOCASE ASC, t.id ASC"),
        ("album", true) => Some("t.album COLLATE NOCASE DESC, t.id ASC"),
        ("genre", false) => Some("COALESCE(t.genre, '') COLLATE NOCASE ASC, t.id ASC"),
        ("genre", true) => Some("COALESCE(t.genre, '') COLLATE NOCASE DESC, t.id ASC"),
        ("year", false) => Some("COALESCE(t.year, 0) ASC, t.id ASC"),
        ("year", true) => Some("COALESCE(t.year, 0) DESC, t.id ASC"),
        ("duration", false) => Some("t.duration_secs ASC, t.id ASC"),
        ("duration", true) => Some("t.duration_secs DESC, t.id ASC"),
        ("playCount", false) => Some("COALESCE(s.play_count, 0) ASC, t.id ASC"),
        ("playCount", true) => Some("COALESCE(s.play_count, 0) DESC, t.id ASC"),
        ("lastPlayed", false) => Some("COALESCE(s.last_played, 0) ASC, t.id ASC"),
        ("lastPlayed", true) => Some("COALESCE(s.last_played, 0) DESC, t.id ASC"),
        ("dateAdded", false) => Some("t.first_seen_at ASC, t.id ASC"),
        ("dateAdded", true) => Some("t.first_seen_at DESC, t.id ASC"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use serde_json::json;

    fn database() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory database");
        conn.execute_batch(
            "CREATE TABLE tracks (
               id INTEGER PRIMARY KEY,
               path TEXT NOT NULL UNIQUE,
               title TEXT NOT NULL DEFAULT '',
               artist TEXT NOT NULL DEFAULT '',
               album TEXT NOT NULL DEFAULT '',
               genre TEXT,
               duration_secs INTEGER NOT NULL DEFAULT 0,
               date_added INTEGER NOT NULL DEFAULT 0,
               first_seen_at INTEGER NOT NULL DEFAULT 0,
               year INTEGER,
               track_number INTEGER,
               has_cover INTEGER NOT NULL DEFAULT 0,
               sample_rate INTEGER,
               bit_depth INTEGER,
               track_gain_db REAL,
               track_peak REAL,
               file_size INTEGER,
               mtime_ns INTEGER
             );
             CREATE TABLE stats (
               track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
               play_count INTEGER NOT NULL DEFAULT 0,
               last_played INTEGER NOT NULL DEFAULT 0,
               skip_count INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE favorites (
               track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
               position INTEGER NOT NULL DEFAULT 0
             );",
        )
        .expect("create schema");
        conn
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_track(
        conn: &Connection,
        path: &str,
        title: &str,
        artist: &str,
        album: &str,
        genre: Option<&str>,
        duration: i64,
        date_added: i64,
        year: Option<i64>,
        plays: Option<(i64, i64)>,
        favorite: bool,
    ) {
        conn.execute(
            "INSERT INTO tracks
             (path, title, artist, album, genre, duration_secs, date_added, first_seen_at, year)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8)",
            params![path, title, artist, album, genre, duration, date_added, year],
        )
        .expect("insert track");
        if let Some((play_count, last_played)) = plays {
            conn.execute(
                "INSERT INTO stats(track_id, play_count, last_played)
                 SELECT id, ?2, ?3 FROM tracks WHERE path = ?1",
                params![path, play_count, last_played],
            )
            .expect("insert stats");
        }
        if favorite {
            conn.execute(
                "INSERT INTO favorites(track_id, position)
                 SELECT id, 0 FROM tracks WHERE path = ?1",
                params![path],
            )
            .expect("insert favorite");
        }
    }

    #[test]
    fn compiles_values_as_parameters_and_escapes_like_wildcards() {
        let rules = json!({
            "match": "all",
            "conditions": [{
                "field": "title",
                "op": "contains",
                "value": "%_'); DROP TABLE tracks; --"
            }]
        });
        let compiled = compile_rules(&rules, 123);

        assert!(!compiled.where_sql.contains("DROP TABLE"));
        assert_eq!(
            compiled.params,
            vec![SqlValue::Text("%^%^_'); DROP TABLE tracks; --%".into())]
        );
    }

    #[test]
    fn evaluates_all_rule_types_in_sql() {
        let conn = database();
        let now = now_ms();
        insert_track(
            &conn,
            "a.flac",
            "100% Real",
            "Alpha",
            "First",
            Some("Rock"),
            180,
            (now / 1000) - 86_400,
            Some(2024),
            Some((8, now - 3_600_000)),
            true,
        );
        insert_track(
            &conn,
            "b.flac",
            "Other",
            "Beta",
            "Second",
            None,
            420,
            (now / 1000) - 100 * 86_400,
            None,
            None,
            false,
        );

        let rules = json!({
            "match": "all",
            "conditions": [
                {"field": "title", "op": "contains", "value": "%"},
                {"field": "genre", "op": "is", "value": "rock"},
                {"field": "year", "op": "gte", "value": "2020"},
                {"field": "duration", "op": "lt", "value": 200},
                {"field": "playCount", "op": "gt", "value": 5},
                {"field": "lastPlayed", "op": "inLast", "value": 1},
                {"field": "dateAdded", "op": "inLast", "value": 2},
                {"field": "favorite", "op": "isTrue"}
            ]
        });
        let tracks = smart_eval(&conn, &rules, "none", "asc", 0).expect("evaluate rules");

        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].path, "a.flac");
    }

    #[test]
    fn supports_any_sort_limit_and_count_without_materializing_all_rows() {
        let conn = database();
        insert_track(
            &conn,
            "a.flac",
            "Alpha",
            "One",
            "A",
            None,
            100,
            10,
            Some(2000),
            Some((2, 100)),
            false,
        );
        insert_track(
            &conn,
            "b.flac",
            "Beta",
            "Two",
            "B",
            None,
            200,
            20,
            Some(2010),
            Some((9, 200)),
            true,
        );
        insert_track(
            &conn,
            "c.flac",
            "Gamma",
            "Three",
            "C",
            None,
            300,
            30,
            Some(2020),
            None,
            false,
        );
        let rules = json!({
            "match": "any",
            "conditions": [
                {"field": "favorite", "op": "isTrue"},
                {"field": "year", "op": "gte", "value": 2000}
            ]
        });

        let tracks = smart_eval(&conn, &rules, "playCount", "desc", 2).expect("evaluate rules");
        assert_eq!(
            tracks
                .iter()
                .map(|track| track.path.as_str())
                .collect::<Vec<_>>(),
            vec!["b.flac", "a.flac"]
        );
        assert_eq!(smart_count(&conn, &rules, 2).expect("count rules"), 2);
        assert_eq!(smart_count(&conn, &rules, 0).expect("count rules"), 3);
        let random = smart_eval(&conn, &rules, "random", "asc", 2).expect("random window");
        assert_eq!(random.len(), 2);
        assert_ne!(random[0].path, random[1].path);
    }

    #[test]
    fn unknown_fields_and_sort_keys_cannot_inject_sql() {
        let conn = database();
        insert_track(
            &conn,
            "safe.flac",
            "Safe",
            "Artist",
            "Album",
            None,
            1,
            1,
            None,
            None,
            false,
        );
        let rules = json!({
            "match": "all",
            "conditions": [{"field": "t.path); DROP TABLE tracks; --", "op": "is", "value": "x"}]
        });

        let tracks = smart_eval(&conn, &rules, "title; DROP TABLE tracks; --", "desc", 0)
            .expect("unknown keys remain harmless");
        assert_eq!(tracks.len(), 1);
        let table_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM tracks", [], |row| row.get(0))
            .expect("tracks table remains");
        assert_eq!(table_count, 1);
    }
}
