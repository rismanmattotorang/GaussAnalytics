//! Chart generation: pick a sensible Plotly chart for a [`DataFrame`] and emit
//! a renderer-ready figure spec. Mirrors `gauss/integrations/plotly`.
//!
//! Heuristics (kept deliberately simple for phase 2):
//! - 1 numeric column            → histogram
//! - 2 numeric columns           → scatter
//! - 1 categorical + 1 numeric   → bar
//! - anything else               → table

use gauss_engine::dataframe::DataFrame;
use serde_json::{json, Value};

pub mod recommend;
pub use recommend::{column_kinds, measure_columns, recommend, ChartRecommendation, ColumnKind};

/// A generated chart: the Plotly `chart_type` and the figure spec.
pub struct Chart {
    pub chart_type: String,
    pub figure: Value,
}

fn is_numeric_column(df: &DataFrame, col: usize) -> bool {
    let mut saw_number = false;
    for row in &df.rows {
        match row.get(col) {
            Some(Value::Number(_)) => saw_number = true,
            Some(Value::Null) | None => {}
            _ => return false,
        }
    }
    saw_number
}

fn cell_string(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn cell_number(v: Option<&Value>) -> f64 {
    v.and_then(Value::as_f64).unwrap_or(f64::NAN)
}

fn column_strings(df: &DataFrame, col: usize) -> Vec<String> {
    df.rows.iter().map(|r| cell_string(r.get(col))).collect()
}

fn column_numbers(df: &DataFrame, col: usize) -> Vec<f64> {
    df.rows.iter().map(|r| cell_number(r.get(col))).collect()
}

/// Generate a chart for `df`.
pub fn generate_chart(df: &DataFrame, title: Option<&str>) -> Chart {
    let title = title.unwrap_or("Chart");
    let numeric: Vec<usize> = (0..df.column_count())
        .filter(|&c| is_numeric_column(df, c))
        .collect();

    // 1 numeric column → histogram.
    if df.column_count() == 1 && numeric.len() == 1 {
        let figure = json!({
            "data": [{ "type": "histogram", "x": column_numbers(df, 0) }],
            "layout": { "title": { "text": title }, "xaxis": { "title": df.columns[0] } }
        });
        return Chart {
            chart_type: "histogram".into(),
            figure,
        };
    }

    // Exactly 2 numeric columns → scatter.
    if df.column_count() == 2 && numeric.len() == 2 {
        let figure = json!({
            "data": [{
                "type": "scatter", "mode": "markers",
                "x": column_numbers(df, 0), "y": column_numbers(df, 1)
            }],
            "layout": {
                "title": { "text": title },
                "xaxis": { "title": df.columns[0] },
                "yaxis": { "title": df.columns[1] }
            }
        });
        return Chart {
            chart_type: "scatter".into(),
            figure,
        };
    }

    // 1 categorical + 1 numeric → bar.
    if df.column_count() == 2 && numeric.len() == 1 {
        let num_col = numeric[0];
        let cat_col = 1 - num_col;
        let figure = json!({
            "data": [{
                "type": "bar",
                "x": column_strings(df, cat_col),
                "y": column_numbers(df, num_col)
            }],
            "layout": {
                "title": { "text": title },
                "xaxis": { "title": df.columns[cat_col] },
                "yaxis": { "title": df.columns[num_col] }
            }
        });
        return Chart {
            chart_type: "bar".into(),
            figure,
        };
    }

    // Fallback: render the data as a table.
    let columns: Vec<String> = df.columns.clone();
    let cells: Vec<Vec<String>> = (0..df.column_count())
        .map(|c| column_strings(df, c))
        .collect();
    let figure = json!({
        "data": [{
            "type": "table",
            "header": { "values": columns },
            "cells": { "values": cells }
        }],
        "layout": { "title": { "text": title } }
    });
    Chart {
        chart_type: "table".into(),
        figure,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn df(columns: &[&str], rows: Vec<Vec<Value>>) -> DataFrame {
        DataFrame::new(
            columns
                .iter()
                .map(std::string::ToString::to_string)
                .collect(),
            rows,
        )
    }

    #[test]
    fn categorical_and_numeric_is_bar() {
        let d = df(
            &["name", "value"],
            vec![vec![json!("A"), json!(10.0)], vec![json!("B"), json!(20.0)]],
        );
        let c = generate_chart(&d, Some("T"));
        assert_eq!(c.chart_type, "bar");
        assert_eq!(c.figure["data"][0]["x"][0], "A");
        assert_eq!(c.figure["data"][0]["y"][1], 20.0);
        assert_eq!(c.figure["layout"]["title"]["text"], "T");
    }

    #[test]
    fn two_numeric_is_scatter() {
        let d = df(
            &["x", "y"],
            vec![vec![json!(1.0), json!(2.0)], vec![json!(3.0), json!(4.0)]],
        );
        assert_eq!(generate_chart(&d, None).chart_type, "scatter");
    }

    #[test]
    fn single_numeric_is_histogram() {
        let d = df(&["v"], vec![vec![json!(1.0)], vec![json!(2.0)]]);
        assert_eq!(generate_chart(&d, None).chart_type, "histogram");
    }

    #[test]
    fn otherwise_table() {
        let d = df(
            &["a", "b", "c"],
            vec![vec![json!("x"), json!("y"), json!("z")]],
        );
        assert_eq!(generate_chart(&d, None).chart_type, "table");
    }
}
