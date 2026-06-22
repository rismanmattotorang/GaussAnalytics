//! `visualize_data` tool: read a CSV (typically produced by `run_sql`), pick a
//! chart, and stream a `ChartComponent`. Mirrors `gauss/tools/visualize_data.py`.

use async_trait::async_trait;
use gauss_engine::components::{RichComponent, UiComponent};
use gauss_engine::context::{ToolContext, ToolResult};
use gauss_engine::dataframe::DataFrame;
use gauss_engine::tool::Tool;
use gauss_engine::traits::FileSystem;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct VisualizeDataArgs {
    /// The CSV filename to visualize (e.g. a file produced by `run_sql`).
    pub filename: String,
    /// Optional chart title.
    #[serde(default)]
    pub title: Option<String>,
}

/// Reads a CSV file and renders an appropriate chart.
pub struct VisualizeDataTool {
    file_system: Arc<dyn FileSystem>,
    access_groups: Vec<String>,
}

impl VisualizeDataTool {
    pub fn new(file_system: Arc<dyn FileSystem>) -> Self {
        Self {
            file_system,
            access_groups: vec!["user".into(), "admin".into()],
        }
    }
}

/// Parse CSV text into a [`DataFrame`], inferring numeric cells.
pub fn parse_csv(text: &str) -> Result<DataFrame, String> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_reader(text.as_bytes());
    let columns: Vec<String> = reader
        .headers()
        .map_err(|e| format!("csv headers: {e}"))?
        .iter()
        .map(str::to_string)
        .collect();
    let mut rows: Vec<Vec<Value>> = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|e| format!("csv row: {e}"))?;
        let row = record
            .iter()
            .map(|field| match field.parse::<f64>() {
                Ok(n) => json!(n),
                Err(_) => json!(field),
            })
            .collect();
        rows.push(row);
    }
    Ok(DataFrame::new(columns, rows))
}

#[async_trait]
impl Tool for VisualizeDataTool {
    type Args = VisualizeDataArgs;

    fn name(&self) -> &str {
        "visualize_data"
    }
    fn description(&self) -> &str {
        "Read a CSV file (such as one produced by run_sql) and render an appropriate chart \
         (bar, scatter, histogram, or table)."
    }
    fn access_groups(&self) -> Vec<String> {
        self.access_groups.clone()
    }

    async fn execute(&self, context: &ToolContext, args: VisualizeDataArgs) -> ToolResult {
        let text = match self.file_system.read_file(&args.filename, context).await {
            Ok(t) => t,
            Err(e) => return ToolResult::error(format!("Could not read {}: {e}", args.filename)),
        };
        let df = match parse_csv(&text) {
            Ok(df) => df,
            Err(e) => return ToolResult::error(format!("Could not parse CSV: {e}")),
        };
        let chart = gauss_chart::generate_chart(&df, args.title.as_deref());
        let component = UiComponent::new(RichComponent::chart(
            chart.chart_type.clone(),
            chart.figure,
            args.title.clone(),
        ));
        ToolResult::success(format!(
            "Rendered a {} chart from {} ({} rows).",
            chart.chart_type,
            args.filename,
            df.row_count()
        ))
        .with_ui(component)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_csv_infers_numbers() {
        let df = parse_csv("name,value\nAcme,10\nGlobex,20.5\n").unwrap();
        assert_eq!(df.columns, vec!["name", "value"]);
        assert_eq!(df.row_count(), 2);
        assert_eq!(df.rows[0][0], json!("Acme"));
        assert_eq!(df.rows[1][1], json!(20.5));
    }
}
