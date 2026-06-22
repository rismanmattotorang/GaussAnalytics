//! Deterministic chart recommendation.
//!
//! Given a [`DataFrame`], pick the most informative chart and emit a **Vega-Lite
//! v5** spec grounded *only* in the columns that actually exist in the result.
//!
//! This is GaussAnalytics' answer to WrenAI's "Text-to-Chart": where WrenAI
//! spends an extra LLM round-trip to author a Vega-Lite spec (slow, costs
//! tokens, and can reference columns the query never returned), we choose the
//! chart from the data's *shape* — column kinds and cardinality — in-process.
//! The result is instant, free, reproducible, unit-testable, and structurally
//! incapable of hallucinating a field. We still emit a standard Vega-Lite spec
//! (so the output interoperates with the wider ecosystem) plus a compact
//! `encoding` the dependency-free, no-CDN renderers can draw directly.

use gauss_engine::dataframe::DataFrame;
use serde_json::{json, Map, Value};

/// How many rows to embed in a chart spec (charts past this are unreadable).
const MAX_CHART_ROWS: usize = 500;
/// Cardinality at or below which a single-measure breakdown reads well as a pie.
const PIE_MAX_SLICES: usize = 6;
/// Above this distinct-category count a bar chart is noise, not signal.
const BAR_MAX_BARS: usize = 50;

/// The semantic kind of a column, inferred from its name and sampled values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    Quantitative,
    Temporal,
    Categorical,
}

impl ColumnKind {
    /// The Vega-Lite measurement type string.
    fn vega_type(self) -> &'static str {
        match self {
            ColumnKind::Quantitative => "quantitative",
            ColumnKind::Temporal => "temporal",
            ColumnKind::Categorical => "nominal",
        }
    }
}

/// A recommended chart: a Vega-Lite spec plus compact hints for inline renderers.
#[derive(Debug, Clone)]
pub struct ChartRecommendation {
    /// One of: number, bar, grouped_bar, line, multi_line, pie, scatter.
    pub chart_type: String,
    pub title: String,
    /// Why this chart was chosen — surfaced to the user as a one-liner.
    pub reason: String,
    /// A full Vega-Lite v5 specification (inline data, real columns only).
    pub vega_lite: Value,
    /// Compact encoding for dependency-free renderers (web/TUI), avoiding a
    /// Vega runtime: `{ x, y, series? }` column names.
    pub encoding: Value,
}

impl ChartRecommendation {
    /// Serialize to the `chart` object embedded in a dataframe component.
    pub fn to_json(&self) -> Value {
        json!({
            "chart_type": self.chart_type,
            "title": self.title,
            "reason": self.reason,
            "vega_lite": self.vega_lite,
            "encoding": self.encoding,
        })
    }
}

/// Does this string look like an ISO-ish date/datetime? (cheap, no chrono).
fn looks_temporal_value(s: &str) -> bool {
    let b = s.as_bytes();
    // YYYY-MM-DD or YYYY/MM/DD prefix.
    b.len() >= 10
        && b[0..4].iter().all(u8::is_ascii_digit)
        && (b[4] == b'-' || b[4] == b'/')
        && b[5..7].iter().all(u8::is_ascii_digit)
        && (b[7] == b'-' || b[7] == b'/')
        && b[8..10].iter().all(u8::is_ascii_digit)
}

/// Token-based temporal-name detection. Matching whole `_`/non-alphanumeric
/// tokens (not substrings) avoids false positives like `life`+`time` in
/// `lifetime_value` or `pay`+`day` in `payday_amount`.
fn name_suggests_temporal(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    if n.ends_with("_at") {
        return true;
    }
    const TOKENS: &[&str] = &[
        "date",
        "datetime",
        "time",
        "timestamp",
        "year",
        "month",
        "day",
        "week",
        "quarter",
        "period",
        "created",
        "updated",
        "modified",
    ];
    n.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|tok| TOKENS.contains(&tok))
}

/// Heuristic: a numeric column that is really an identifier, not a measure
/// (`id`, `customer_id`, …). Such columns make misleading totals and axes.
fn is_likely_id(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == "id" || n.ends_with("_id") || n.ends_with("_key") || n == "key"
}

/// The quantitative columns that read as real **measures** — quantitative kind,
/// excluding identifier-like columns. Shared by chart selection and summaries
/// so both ignore primary/foreign keys when picking what to total or plot.
pub fn measure_columns(df: &DataFrame) -> Vec<usize> {
    let kinds = infer_kinds(df);
    (0..df.column_count())
        .filter(|&c| {
            kinds[c] == ColumnKind::Quantitative
                && !is_likely_id(df.columns.get(c).map(String::as_str).unwrap_or(""))
        })
        .collect()
}

/// Infer each column's kind from its name and a sample of its values.
fn infer_kinds(df: &DataFrame) -> Vec<ColumnKind> {
    (0..df.column_count())
        .map(|c| {
            let mut saw_value = false;
            let mut all_numbers = true;
            let mut temporal_strings = true;
            let mut saw_string = false;
            for row in df.rows.iter().take(200) {
                match row.get(c) {
                    Some(Value::Null) | None => {}
                    Some(Value::Number(_)) => saw_value = true,
                    Some(Value::String(s)) => {
                        saw_value = true;
                        all_numbers = false;
                        saw_string = true;
                        if !looks_temporal_value(s) {
                            temporal_strings = false;
                        }
                    }
                    Some(_) => {
                        saw_value = true;
                        all_numbers = false;
                        temporal_strings = false;
                    }
                }
            }
            let name = df.columns.get(c).map(String::as_str).unwrap_or("");
            if saw_value && all_numbers {
                // Numeric, but a numeric "year" column reads better as temporal.
                if name_suggests_temporal(name) {
                    ColumnKind::Temporal
                } else {
                    ColumnKind::Quantitative
                }
            } else if (saw_string && temporal_strings && saw_value)
                || (name_suggests_temporal(name) && !all_numbers)
            {
                ColumnKind::Temporal
            } else {
                ColumnKind::Categorical
            }
        })
        .collect()
}

/// Infer the [`ColumnKind`] of every column (name + sampled values).
pub fn column_kinds(df: &DataFrame) -> Vec<ColumnKind> {
    infer_kinds(df)
}

fn distinct_count(df: &DataFrame, col: usize) -> usize {
    let mut seen = std::collections::HashSet::new();
    for row in &df.rows {
        let key = match row.get(col) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Null) | None => String::new(),
            Some(other) => other.to_string(),
        };
        seen.insert(key);
    }
    seen.len()
}

/// Records (column→value maps) capped at [`MAX_CHART_ROWS`] for embedding.
fn capped_records(df: &DataFrame) -> Vec<Map<String, Value>> {
    df.to_records().into_iter().take(MAX_CHART_ROWS).collect()
}

fn vega(title: &str, mark: Value, encoding: Value, records: Vec<Map<String, Value>>) -> Value {
    json!({
        "$schema": "https://vega.github.io/schema/vega-lite/v5.json",
        "title": title,
        "data": { "values": records.into_iter().map(Value::Object).collect::<Vec<_>>() },
        "mark": mark,
        "encoding": encoding,
    })
}

fn field(name: &str, kind: ColumnKind) -> Value {
    json!({ "field": name, "type": kind.vega_type() })
}

/// Recommend a chart for `df`, or `None` when a table is the honest display
/// (no rows, no measure, or too many categories to plot legibly).
pub fn recommend(df: &DataFrame, title: Option<&str>) -> Option<ChartRecommendation> {
    if df.rows.is_empty() || df.column_count() == 0 {
        return None;
    }
    let title = title.unwrap_or("Chart").to_string();
    let kinds = infer_kinds(df);
    let cols = &df.columns;

    let quant: Vec<usize> = measure_columns(df);
    let temporal: Vec<usize> = (0..df.column_count())
        .filter(|&c| kinds[c] == ColumnKind::Temporal)
        .collect();
    let categ: Vec<usize> = (0..df.column_count())
        .filter(|&c| kinds[c] == ColumnKind::Categorical)
        .collect();

    // A single scalar → headline "number" tile.
    if df.rows.len() == 1 && quant.len() == 1 && df.column_count() == 1 {
        let m = quant[0];
        return Some(ChartRecommendation {
            chart_type: "number".into(),
            title: title.clone(),
            reason: format!("A single value — shown as a headline metric ({}).", cols[m]),
            vega_lite: json!({
                "$schema": "https://vega.github.io/schema/vega-lite/v5.json",
                "title": title,
                "data": { "values": capped_records(df).into_iter().map(Value::Object).collect::<Vec<_>>() },
                "mark": { "type": "text", "fontSize": 48 },
                "encoding": { "text": field(&cols[m], ColumnKind::Quantitative) }
            }),
            encoding: json!({ "value": cols[m] }),
        });
    }

    // Time series: temporal x with one or more measures → line / multi-line.
    if temporal.len() == 1 && !quant.is_empty() {
        let x = temporal[0];
        // A categorical series splits one measure into multiple lines.
        if quant.len() == 1 && categ.len() == 1 && distinct_count(df, categ[0]) <= 12 {
            let (y, s) = (quant[0], categ[0]);
            return Some(ChartRecommendation {
                chart_type: "multi_line".into(),
                title: title.clone(),
                reason: format!("{} over {}, split by {}.", cols[y], cols[x], cols[s]),
                vega_lite: vega(
                    &title,
                    json!({ "type": "line", "point": true }),
                    json!({
                        "x": field(&cols[x], ColumnKind::Temporal),
                        "y": field(&cols[y], ColumnKind::Quantitative),
                        "color": field(&cols[s], ColumnKind::Categorical),
                    }),
                    capped_records(df),
                ),
                encoding: json!({ "x": cols[x], "y": cols[y], "series": cols[s] }),
            });
        }
        let y = quant[0];
        let chart_type = if quant.len() >= 2 {
            "multi_line"
        } else {
            "line"
        };
        return Some(ChartRecommendation {
            chart_type: chart_type.into(),
            title: title.clone(),
            reason: format!("{} trend over {}.", cols[y], cols[x]),
            vega_lite: vega(
                &title,
                json!({ "type": "line", "point": true }),
                json!({
                    "x": field(&cols[x], ColumnKind::Temporal),
                    "y": field(&cols[y], ColumnKind::Quantitative),
                }),
                capped_records(df),
            ),
            encoding: json!({ "x": cols[x], "y": cols[y] }),
        });
    }

    // Two categoricals + a measure → grouped bar.
    if categ.len() == 2 && quant.len() == 1 && distinct_count(df, categ[0]) <= BAR_MAX_BARS {
        let (x, s, y) = (categ[0], categ[1], quant[0]);
        return Some(ChartRecommendation {
            chart_type: "grouped_bar".into(),
            title: title.clone(),
            reason: format!("{} by {}, grouped by {}.", cols[y], cols[x], cols[s]),
            vega_lite: vega(
                &title,
                json!({ "type": "bar" }),
                json!({
                    "x": field(&cols[x], ColumnKind::Categorical),
                    "y": field(&cols[y], ColumnKind::Quantitative),
                    "color": field(&cols[s], ColumnKind::Categorical),
                    "xOffset": field(&cols[s], ColumnKind::Categorical),
                }),
                capped_records(df),
            ),
            encoding: json!({ "x": cols[x], "y": cols[y], "series": cols[s] }),
        });
    }

    // One category + one measure → pie (few slices) or bar.
    if categ.len() == 1 && quant.len() == 1 {
        let (cat, y) = (categ[0], quant[0]);
        let slices = distinct_count(df, cat);
        if slices <= PIE_MAX_SLICES && df.rows.len() <= PIE_MAX_SLICES {
            return Some(ChartRecommendation {
                chart_type: "pie".into(),
                title: title.clone(),
                reason: format!("{} share across {} {}.", cols[y], slices, cols[cat]),
                vega_lite: vega(
                    &title,
                    json!({ "type": "arc" }),
                    json!({
                        "theta": field(&cols[y], ColumnKind::Quantitative),
                        "color": field(&cols[cat], ColumnKind::Categorical),
                    }),
                    capped_records(df),
                ),
                encoding: json!({ "x": cols[cat], "y": cols[y] }),
            });
        }
        if slices <= BAR_MAX_BARS {
            return Some(ChartRecommendation {
                chart_type: "bar".into(),
                title: title.clone(),
                reason: format!("{} by {}.", cols[y], cols[cat]),
                vega_lite: vega(
                    &title,
                    json!({ "type": "bar" }),
                    json!({
                        "x": field(&cols[cat], ColumnKind::Categorical),
                        "y": field(&cols[y], ColumnKind::Quantitative),
                    }),
                    capped_records(df),
                ),
                encoding: json!({ "x": cols[cat], "y": cols[y] }),
            });
        }
        return None; // too many categories — a table is more honest.
    }

    // One category + several measures → grouped bar across measures.
    if categ.len() == 1 && quant.len() >= 2 && distinct_count(df, categ[0]) <= BAR_MAX_BARS {
        let cat = categ[0];
        let y = quant[0];
        return Some(ChartRecommendation {
            chart_type: "bar".into(),
            title: title.clone(),
            reason: format!(
                "{} by {} (first of {} measures).",
                cols[y],
                cols[cat],
                quant.len()
            ),
            vega_lite: vega(
                &title,
                json!({ "type": "bar" }),
                json!({
                    "x": field(&cols[cat], ColumnKind::Categorical),
                    "y": field(&cols[y], ColumnKind::Quantitative),
                }),
                capped_records(df),
            ),
            encoding: json!({ "x": cols[cat], "y": cols[y] }),
        });
    }

    // Exactly two measures, nothing else → scatter (correlation).
    if quant.len() == 2 && categ.is_empty() && temporal.is_empty() {
        let (x, y) = (quant[0], quant[1]);
        return Some(ChartRecommendation {
            chart_type: "scatter".into(),
            title: title.clone(),
            reason: format!("Relationship between {} and {}.", cols[x], cols[y]),
            vega_lite: vega(
                &title,
                json!({ "type": "point" }),
                json!({
                    "x": field(&cols[x], ColumnKind::Quantitative),
                    "y": field(&cols[y], ColumnKind::Quantitative),
                }),
                capped_records(df),
            ),
            encoding: json!({ "x": cols[x], "y": cols[y] }),
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn df(columns: &[&str], rows: Vec<Vec<Value>>) -> DataFrame {
        DataFrame::new(columns.iter().map(ToString::to_string).collect(), rows)
    }

    #[test]
    fn category_plus_measure_is_bar() {
        let d = df(
            &["country", "revenue"],
            vec![
                vec![json!("US"), json!(100)],
                vec![json!("DE"), json!(80)],
                vec![json!("UK"), json!(60)],
                vec![json!("FR"), json!(40)],
                vec![json!("JP"), json!(30)],
                vec![json!("CN"), json!(20)],
                vec![json!("IN"), json!(10)],
            ],
        );
        let r = recommend(&d, Some("Rev")).unwrap();
        assert_eq!(r.chart_type, "bar");
        assert_eq!(r.encoding["x"], "country");
        assert_eq!(r.encoding["y"], "revenue");
        assert_eq!(r.vega_lite["mark"]["type"], "bar");
        // Vega-Lite references only real columns.
        assert_eq!(r.vega_lite["encoding"]["x"]["field"], "country");
    }

    #[test]
    fn few_slices_is_pie() {
        let d = df(
            &["status", "n"],
            vec![
                vec![json!("paid"), json!(3)],
                vec![json!("pending"), json!(1)],
                vec![json!("refunded"), json!(1)],
            ],
        );
        let r = recommend(&d, None).unwrap();
        assert_eq!(r.chart_type, "pie");
        assert_eq!(r.vega_lite["mark"]["type"], "arc");
        assert_eq!(r.vega_lite["encoding"]["theta"]["field"], "n");
    }

    #[test]
    fn temporal_plus_measure_is_line() {
        let d = df(
            &["month", "sales"],
            vec![
                vec![json!("2024-01-01"), json!(10)],
                vec![json!("2024-02-01"), json!(20)],
                vec![json!("2024-03-01"), json!(15)],
            ],
        );
        let r = recommend(&d, None).unwrap();
        assert_eq!(r.chart_type, "line");
        assert_eq!(r.vega_lite["encoding"]["x"]["type"], "temporal");
    }

    #[test]
    fn temporal_with_series_is_multi_line() {
        let d = df(
            &["day", "region", "sales"],
            vec![
                vec![json!("2024-01-01"), json!("US"), json!(10)],
                vec![json!("2024-01-01"), json!("EU"), json!(8)],
                vec![json!("2024-01-02"), json!("US"), json!(12)],
                vec![json!("2024-01-02"), json!("EU"), json!(9)],
            ],
        );
        let r = recommend(&d, None).unwrap();
        assert_eq!(r.chart_type, "multi_line");
        assert_eq!(r.encoding["series"], "region");
    }

    #[test]
    fn two_categoricals_and_measure_is_grouped_bar() {
        let d = df(
            &["region", "segment", "revenue"],
            vec![
                vec![json!("US"), json!("SMB"), json!(10)],
                vec![json!("US"), json!("Ent"), json!(40)],
                vec![json!("EU"), json!("SMB"), json!(8)],
            ],
        );
        let r = recommend(&d, None).unwrap();
        assert_eq!(r.chart_type, "grouped_bar");
        assert_eq!(r.vega_lite["encoding"]["xOffset"]["field"], "segment");
    }

    #[test]
    fn two_measures_is_scatter() {
        let d = df(
            &["height", "weight"],
            vec![vec![json!(1.0), json!(2.0)], vec![json!(3.0), json!(4.0)]],
        );
        assert_eq!(recommend(&d, None).unwrap().chart_type, "scatter");
    }

    #[test]
    fn single_value_is_number() {
        let d = df(&["total"], vec![vec![json!(42)]]);
        assert_eq!(recommend(&d, None).unwrap().chart_type, "number");
    }

    #[test]
    fn too_many_categories_is_table() {
        let rows: Vec<Vec<Value>> = (0..80)
            .map(|i| vec![json!(format!("id-{i}")), json!(i)])
            .collect();
        let d = df(&["id", "count"], rows);
        assert!(recommend(&d, None).is_none());
    }

    #[test]
    fn empty_is_none() {
        let d = df(&["a", "b"], vec![]);
        assert!(recommend(&d, None).is_none());
    }

    #[test]
    fn lifetime_value_is_not_temporal() {
        // Regression: "lifetime_value" contains the substring "time" but must
        // NOT be treated as a time axis. country + lifetime_value → bar.
        let d = df(
            &["country", "lifetime_value"],
            vec![
                vec![json!("US"), json!(152000.0)],
                vec![json!("DE"), json!(98000.0)],
                vec![json!("UK"), json!(210000.0)],
                vec![json!("FR"), json!(45000.0)],
                vec![json!("JP"), json!(30000.0)],
                vec![json!("CN"), json!(20000.0)],
                vec![json!("IN"), json!(10000.0)],
            ],
        );
        let r = recommend(&d, None).unwrap();
        assert_eq!(r.chart_type, "bar", "got {}", r.chart_type);
        assert_eq!(r.encoding["y"], "lifetime_value");
    }

    #[test]
    fn id_columns_are_not_measures() {
        // A raw table of (id, name) has no real measure → table, not a chart.
        let d = df(
            &["id", "name"],
            vec![
                vec![json!(1), json!("Acme")],
                vec![json!(2), json!("Globex")],
            ],
        );
        assert!(recommend(&d, None).is_none());
        assert!(measure_columns(&d).is_empty());
    }

    #[test]
    fn created_at_is_temporal_token() {
        let d = df(
            &["created_at", "revenue"],
            vec![
                vec![json!("2024-01-01"), json!(10)],
                vec![json!("2024-02-01"), json!(20)],
            ],
        );
        assert_eq!(recommend(&d, None).unwrap().chart_type, "line");
    }

    #[test]
    fn numeric_year_is_treated_as_temporal() {
        let d = df(
            &["year", "sales"],
            vec![
                vec![json!(2021), json!(10)],
                vec![json!(2022), json!(20)],
                vec![json!(2023), json!(30)],
            ],
        );
        let r = recommend(&d, None).unwrap();
        assert_eq!(r.chart_type, "line");
    }
}
