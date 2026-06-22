//! GenBI result intelligence for GaussAnalytics.
//!
//! Given a query result ([`DataFrame`]), produce — entirely in-process and
//! deterministically — the three things a generative-BI surface wants to show
//! next to a table: a **recommended chart**, a **plain-language summary**, and
//! **grounded follow-up questions**.
//!
//! This is GaussAnalytics' take on WrenAI's "GenBI" result panel, but without
//! the extra LLM round-trips WrenAI spends on chart authoring, summarization,
//! and suggestion generation. Because every figure here is computed from the
//! actual returned rows:
//!
//! - the **summary never misstates a number** (a classic LLM-summary failure),
//! - the **suggestions only reference columns that exist**, and
//! - the whole panel is **instant, free, reproducible, and unit-tested**.
//!
//! An LLM can still narrate on top of this, but the figures are ground truth.

use gauss_chart::{column_kinds, measure_columns, recommend, ChartRecommendation, ColumnKind};
use gauss_engine::dataframe::DataFrame;
use serde_json::{json, Value};

/// The full GenBI panel for one result.
pub struct ResultInsights {
    /// Recommended chart, when the data has a plottable shape.
    pub chart: Option<ChartRecommendation>,
    /// One- or two-sentence plain-language summary of the result.
    pub summary: String,
    /// Grounded next questions the user might ask.
    pub suggestions: Vec<String>,
}

impl ResultInsights {
    /// Serialize the summary, suggestions, and chart for embedding in a
    /// dataframe UI component (`data.summary`, `data.suggestions`, `data.chart`).
    pub fn to_json(&self) -> Value {
        json!({
            "summary": self.summary,
            "suggestions": self.suggestions,
            "chart": self.chart.as_ref().map(ChartRecommendation::to_json),
        })
    }
}

/// Analyze a result: recommend a chart, summarize it, and suggest follow-ups.
pub fn analyze(df: &DataFrame, title: Option<&str>) -> ResultInsights {
    ResultInsights {
        chart: recommend(df, title),
        summary: summarize(df),
        suggestions: follow_ups(df),
    }
}

fn col_indices(kinds: &[ColumnKind], want: ColumnKind) -> Vec<usize> {
    (0..kinds.len()).filter(|&i| kinds[i] == want).collect()
}

fn fmt_num(n: f64) -> String {
    if n.is_nan() {
        return "n/a".into();
    }
    if (n.fract()).abs() < f64::EPSILON && n.abs() < 1e15 {
        // Integer-valued: group thousands for readability.
        let neg = n < 0.0;
        let mut s = (n.abs() as u64).to_string();
        let mut out = String::new();
        let bytes = s.as_bytes();
        for (i, b) in bytes.iter().enumerate() {
            if i > 0 && (bytes.len() - i).is_multiple_of(3) {
                out.push(',');
            }
            out.push(*b as char);
        }
        s = out;
        if neg {
            format!("-{s}")
        } else {
            s
        }
    } else {
        format!("{n:.2}")
    }
}

fn cell_f64(v: Option<&Value>) -> Option<f64> {
    match v {
        Some(Value::Number(n)) => n.as_f64(),
        _ => None,
    }
}

fn cell_key(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => "(null)".into(),
        Some(other) => other.to_string(),
    }
}

fn pluralize_rows(n: usize) -> String {
    if n == 1 {
        "1 row".into()
    } else {
        format!("{} rows", fmt_num(n as f64))
    }
}

/// A deterministic, never-hallucinating summary of the result.
pub fn summarize(df: &DataFrame) -> String {
    if df.rows.is_empty() {
        return "No rows matched the query.".into();
    }
    let kinds = column_kinds(df);
    let quant = measure_columns(df);
    let temporal = col_indices(&kinds, ColumnKind::Temporal);
    let categ = col_indices(&kinds, ColumnKind::Categorical);

    let mut parts = vec![format!("{}.", pluralize_rows(df.rows.len()))];

    // Headline stats for the first measure.
    if let Some(&m) = quant.first() {
        let vals: Vec<f64> = df.rows.iter().filter_map(|r| cell_f64(r.get(m))).collect();
        if !vals.is_empty() {
            let total: f64 = vals.iter().sum();
            let avg = total / vals.len() as f64;
            let min = vals.iter().cloned().fold(f64::INFINITY, f64::min);
            let max = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let name = &df.columns[m];
            if df.rows.len() == 1 {
                parts.push(format!("{name} is {}.", fmt_num(vals[0])));
            } else {
                parts.push(format!(
                    "{name}: total {}, average {}, ranging {} to {}.",
                    fmt_num(total),
                    fmt_num(avg),
                    fmt_num(min),
                    fmt_num(max)
                ));
            }

            // Leading category by that measure.
            if let Some(&cat) = categ.first() {
                let mut best_key = String::new();
                let mut best_val = f64::NEG_INFINITY;
                let mut agg: std::collections::HashMap<String, f64> =
                    std::collections::HashMap::new();
                for row in &df.rows {
                    let k = cell_key(row.get(cat));
                    let v = cell_f64(row.get(m)).unwrap_or(0.0);
                    *agg.entry(k).or_insert(0.0) += v;
                }
                for (k, v) in agg {
                    if v > best_val {
                        best_val = v;
                        best_key = k;
                    }
                }
                if !best_key.is_empty() {
                    parts.push(format!(
                        "Top {} is {} ({}).",
                        df.columns[cat],
                        best_key,
                        fmt_num(best_val)
                    ));
                }
            }
        }
    } else if let Some(&cat) = categ.first() {
        // No measure: report distinct categories.
        let distinct: std::collections::HashSet<String> =
            df.rows.iter().map(|r| cell_key(r.get(cat))).collect();
        parts.push(format!(
            "{} distinct {}.",
            fmt_num(distinct.len() as f64),
            df.columns[cat]
        ));
    }

    // Time span.
    if let Some(&t) = temporal.first() {
        let mut keys: Vec<String> = df
            .rows
            .iter()
            .map(|r| cell_key(r.get(t)))
            .filter(|s| s != "(null)")
            .collect();
        keys.sort();
        if let (Some(first), Some(last)) = (keys.first(), keys.last()) {
            if first != last {
                parts.push(format!("{} spans {} to {}.", df.columns[t], first, last));
            }
        }
    }

    parts.join(" ")
}

/// Grounded follow-up questions, referencing only columns present in `df`.
pub fn follow_ups(df: &DataFrame) -> Vec<String> {
    if df.rows.is_empty() || df.column_count() == 0 {
        return Vec::new();
    }
    let kinds = column_kinds(df);
    let quant = measure_columns(df);
    let temporal = col_indices(&kinds, ColumnKind::Temporal);
    let categ = col_indices(&kinds, ColumnKind::Categorical);

    let mut out: Vec<String> = Vec::new();

    if let Some(&m) = quant.first() {
        let measure = &df.columns[m];
        for &c in categ.iter().take(2) {
            out.push(format!("Break {} down by {}.", measure, df.columns[c]));
        }
        if let Some(&t) = temporal.first() {
            out.push(format!("Show {} over {}.", measure, df.columns[t]));
        }
        out.push(format!("What are the top 10 by {measure}?"));
    } else if let Some(&c) = categ.first() {
        out.push(format!("How many records per {}?", df.columns[c]));
    }

    out.truncate(4);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn df(columns: &[&str], rows: Vec<Vec<Value>>) -> DataFrame {
        DataFrame::new(columns.iter().map(ToString::to_string).collect(), rows)
    }

    #[test]
    fn empty_result_summary() {
        let d = df(&["a"], vec![]);
        assert_eq!(summarize(&d), "No rows matched the query.");
        assert!(follow_ups(&d).is_empty());
    }

    #[test]
    fn summary_reports_accurate_aggregates() {
        let d = df(
            &["country", "revenue"],
            vec![
                vec![json!("US"), json!(100)],
                vec![json!("DE"), json!(50)],
                vec![json!("US"), json!(50)],
            ],
        );
        let s = summarize(&d);
        // Numbers are computed, not guessed.
        assert!(s.contains("3 rows"), "{s}");
        assert!(s.contains("total 200"), "{s}");
        assert!(s.contains("average 66.67"), "{s}");
        // US (100+50=150) leads DE (50).
        assert!(s.contains("Top country is US (150)"), "{s}");
    }

    #[test]
    fn single_value_summary() {
        let d = df(&["total"], vec![vec![json!(42)]]);
        let s = summarize(&d);
        assert!(s.contains("total is 42"), "{s}");
    }

    #[test]
    fn thousands_are_grouped() {
        let d = df(&["n"], vec![vec![json!(1234567)]]);
        assert!(summarize(&d).contains("1,234,567"));
    }

    #[test]
    fn follow_ups_are_grounded_in_real_columns() {
        let d = df(
            &["month", "region", "sales"],
            vec![
                vec![json!("2024-01-01"), json!("US"), json!(10)],
                vec![json!("2024-02-01"), json!("EU"), json!(20)],
            ],
        );
        let fu = follow_ups(&d);
        assert!(fu.iter().any(|q| q.contains("by region")), "{fu:?}");
        assert!(fu.iter().any(|q| q.contains("over month")), "{fu:?}");
        assert!(fu.iter().any(|q| q.contains("top 10 by sales")), "{fu:?}");
        // Every suggestion mentions a real column name only.
        for q in &fu {
            assert!(
                q.contains("region") || q.contains("month") || q.contains("sales"),
                "ungrounded: {q}"
            );
        }
    }

    #[test]
    fn analyze_bundles_chart_summary_and_suggestions() {
        let d = df(
            &["country", "revenue"],
            vec![
                vec![json!("US"), json!(100)],
                vec![json!("DE"), json!(80)],
                vec![json!("UK"), json!(60)],
            ],
        );
        let ins = analyze(&d, Some("Revenue"));
        assert!(ins.chart.is_some());
        assert!(!ins.summary.is_empty());
        assert!(!ins.suggestions.is_empty());
        let j = ins.to_json();
        assert!(j["summary"].is_string());
        assert!(j["suggestions"].is_array());
        assert_eq!(j["chart"]["chart_type"], "pie"); // 3 slices
    }
}
