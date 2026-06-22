//! The GaussAnalytics **semantic layer**.
//!
//! A `SemanticModel` describes the database in *business* terms — models
//! (logical tables) with described columns and synonyms, reusable relationships
//! (join paths), and metrics — so the LLM is grounded in meaning rather than raw
//! DDL. It also exposes governance metadata (the allowed-table set and PII
//! columns) consumed by the SQL guardrail layer.
//!
//! Authorable as YAML or JSON. This is GaussAnalytics's analog of GaussAnalytics's MDL,
//! implemented independently in Rust.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SemanticModel {
    #[serde(default)]
    pub models: Vec<Model>,
    #[serde(default)]
    pub relationships: Vec<Relationship>,
    #[serde(default)]
    pub metrics: Vec<Metric>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub name: String,
    /// Physical table name; defaults to `name` when omitted.
    #[serde(default)]
    pub table: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub columns: Vec<Column>,
    /// Alternate names a user might use for this entity.
    #[serde(default)]
    pub synonyms: Vec<String>,
}

impl Model {
    pub fn table_name(&self) -> &str {
        self.table.as_deref().unwrap_or(&self.name)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    #[serde(default, rename = "type")]
    pub data_type: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub synonyms: Vec<String>,
    /// Optional SQL expression for a calculated/derived column.
    #[serde(default)]
    pub calculation: Option<String>,
    #[serde(default)]
    pub is_primary_key: bool,
    /// Flags columns that carry personally identifiable information.
    #[serde(default)]
    pub is_pii: bool,
    /// When true, the text-to-SQL pipeline grounds the query in this column's
    /// actual values (so "German" maps to the stored `DE`). Best for low-
    /// cardinality categorical columns (status, country, type, …).
    #[serde(default)]
    pub link_values: bool,
    /// Optional pre-supplied sample values; if empty and `link_values` is set,
    /// the pipeline fetches `SELECT DISTINCT` from the database.
    #[serde(default)]
    pub sample_values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    pub name: String,
    /// `"model.column"` on the source side.
    pub from: String,
    /// `"model.column"` on the target side.
    pub to: String,
    #[serde(default = "default_join")]
    pub join_type: String,
}

fn default_join() -> String {
    "many_to_one".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metric {
    pub name: String,
    pub expression: String,
    #[serde(default)]
    pub description: Option<String>,
}

impl SemanticModel {
    pub fn from_yaml(s: &str) -> Result<Self, String> {
        serde_yaml::from_str(s).map_err(|e| format!("semantic model YAML: {e}"))
    }

    pub fn from_json(s: &str) -> Result<Self, String> {
        serde_json::from_str(s).map_err(|e| format!("semantic model JSON: {e}"))
    }

    /// Load from a `.yaml`/`.yml` or `.json` file (by extension).
    pub fn from_path(path: &str) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
        if path.ends_with(".json") {
            Self::from_json(&text)
        } else {
            Self::from_yaml(&text)
        }
    }

    pub fn model_names(&self) -> Vec<String> {
        self.models.iter().map(|m| m.name.clone()).collect()
    }

    /// Lower-cased physical table names — the SQL allowlist.
    pub fn allowed_tables(&self) -> HashSet<String> {
        self.models
            .iter()
            .map(|m| m.table_name().to_lowercase())
            .collect()
    }

    /// `(table, column)` pairs flagged as PII.
    pub fn pii_columns(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for m in &self.models {
            for c in &m.columns {
                if c.is_pii {
                    out.push((m.table_name().to_string(), c.name.clone()));
                }
            }
        }
        out
    }

    /// Columns flagged for value linking among the named models, as
    /// `(table, column, pre_supplied_sample_values)`.
    pub fn value_linkable_columns(
        &self,
        model_names: &[String],
    ) -> Vec<(String, String, Vec<String>)> {
        let want: HashSet<&str> = model_names.iter().map(String::as_str).collect();
        let mut out = Vec::new();
        for m in self
            .models
            .iter()
            .filter(|m| want.contains(m.name.as_str()))
        {
            for c in &m.columns {
                if c.link_values {
                    out.push((
                        m.table_name().to_string(),
                        c.name.clone(),
                        c.sample_values.clone(),
                    ));
                }
            }
        }
        out
    }

    /// Render the full model as an LLM-friendly, business-grounded context block.
    pub fn render_context(&self) -> String {
        self.render_subset(&self.model_names())
    }

    /// Render only the named models (plus relationships among them and all
    /// metrics) — used after schema linking to keep the prompt focused.
    pub fn render_subset(&self, model_names: &[String]) -> String {
        let want: HashSet<&str> = model_names.iter().map(String::as_str).collect();
        let mut out = String::from("# Data Model\n");

        for m in self
            .models
            .iter()
            .filter(|m| want.contains(m.name.as_str()))
        {
            out.push_str(&format!(
                "\n## Model: {} (table: {})\n",
                m.name,
                m.table_name()
            ));
            if let Some(d) = &m.description {
                out.push_str(&format!("Description: {d}\n"));
            }
            if !m.synonyms.is_empty() {
                out.push_str(&format!("Synonyms: {}\n", m.synonyms.join(", ")));
            }
            out.push_str("Columns:\n");
            for c in &m.columns {
                let ty = c.data_type.as_deref().unwrap_or("");
                let mut tags = Vec::new();
                if c.is_primary_key {
                    tags.push("primary key".to_string());
                }
                if c.is_pii {
                    tags.push("PII".to_string());
                }
                if let Some(expr) = &c.calculation {
                    tags.push(format!("calculated: {expr}"));
                }
                if !c.synonyms.is_empty() {
                    tags.push(format!("synonyms: {}", c.synonyms.join(", ")));
                }
                let tag_str = if tags.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", tags.join("; "))
                };
                let desc = c
                    .description
                    .as_deref()
                    .map(|d| format!(" — {d}"))
                    .unwrap_or_default();
                out.push_str(&format!("- {} ({ty}){tag_str}{desc}\n", c.name));
            }
        }

        // Relationships whose both endpoints are in the subset.
        let rels: Vec<&Relationship> = self
            .relationships
            .iter()
            .filter(|r| {
                let fm = r.from.split('.').next().unwrap_or("");
                let tm = r.to.split('.').next().unwrap_or("");
                want.contains(fm) && want.contains(tm)
            })
            .collect();
        if !rels.is_empty() {
            out.push_str("\n## Relationships (join paths)\n");
            for r in rels {
                out.push_str(&format!("- {} -> {} ({})\n", r.from, r.to, r.join_type));
            }
        }

        if !self.metrics.is_empty() {
            out.push_str("\n## Metrics (reusable calculations)\n");
            for m in &self.metrics {
                let d = m
                    .description
                    .as_deref()
                    .map(|d| format!(" — {d}"))
                    .unwrap_or_default();
                out.push_str(&format!("- {} = {}{d}\n", m.name, m.expression));
            }
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML: &str = r"
models:
  - name: customers
    description: People or companies that buy from us
    synonyms: [clients, accounts]
    columns:
      - name: id
        type: INTEGER
        is_primary_key: true
      - name: name
        type: TEXT
        description: Customer display name
        is_pii: true
      - name: lifetime_value
        type: REAL
        description: Total historical revenue
  - name: orders
    columns:
      - name: id
        type: INTEGER
      - name: customer_id
        type: INTEGER
relationships:
  - name: order_customer
    from: orders.customer_id
    to: customers.id
metrics:
  - name: total_revenue
    expression: SUM(customers.lifetime_value)
    description: Sum of customer lifetime value
";

    #[test]
    fn loads_and_renders() {
        let m = SemanticModel::from_yaml(YAML).unwrap();
        assert_eq!(m.models.len(), 2);
        let ctx = m.render_context();
        assert!(ctx.contains("## Model: customers"));
        assert!(ctx.contains("Synonyms: clients, accounts"));
        assert!(ctx.contains("[PII]") || ctx.contains("PII"));
        assert!(ctx.contains("orders.customer_id -> customers.id"));
        assert!(ctx.contains("total_revenue = SUM"));
    }

    #[test]
    fn governance_metadata() {
        let m = SemanticModel::from_yaml(YAML).unwrap();
        let tables = m.allowed_tables();
        assert!(tables.contains("customers"));
        assert!(tables.contains("orders"));
        assert_eq!(m.pii_columns(), vec![("customers".into(), "name".into())]);
    }

    #[test]
    fn subset_focuses_context() {
        let m = SemanticModel::from_yaml(YAML).unwrap();
        let only_customers = m.render_subset(&["customers".to_string()]);
        assert!(only_customers.contains("## Model: customers"));
        assert!(!only_customers.contains("## Model: orders"));
        // The relationship needs both endpoints, so it is excluded here.
        assert!(!only_customers.contains("join paths"));
    }
}
