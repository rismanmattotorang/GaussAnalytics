//! UI components and their wire-format serialization.
//!
//! The serialized shape is a hard contract with the `<gauss-chat>` frontend
//! (see GAUSSANALYTICS_PORTING_PLAN.md §6.4). Each rich component serializes as:
//! ```jsonc
//! { "id", "type", "lifecycle", "children", "timestamp",
//!   "visible", "interactive", "data": { /* component-specific */ } }
//! ```
//! Enums serialize as their lowercase string values. The three UI-state
//! singletons use fixed IDs (kept as `gauss-*` through phase 1 for frontend
//! compatibility — see the resolved rebrand-timing decision).

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use uuid::Uuid;

/// Fixed IDs for the singleton UI-state components.
pub const STATUS_BAR_ID: &str = "gauss-status-bar";
pub const TASK_TRACKER_ID: &str = "gauss-task-tracker";
pub const CHAT_INPUT_ID: &str = "gauss-chat-input";

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn new_uuid() -> String {
    Uuid::new_v4().to_string()
}

/// The kind of a rich component. `as_str` is the canonical wire value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentType {
    Text,
    Card,
    Container,
    StatusCard,
    DataFrame,
    Chart,
    CodeBlock,
    Notification,
    Badge,
    IconText,
    StatusIndicator,
    ProgressBar,
    ProgressDisplay,
    LogViewer,
    TaskList,
    Button,
    ButtonGroup,
    StatusBarUpdate,
    TaskTrackerUpdate,
    ChatInputUpdate,
}

impl ComponentType {
    pub fn as_str(self) -> &'static str {
        match self {
            ComponentType::Text => "text",
            ComponentType::Card => "card",
            ComponentType::Container => "container",
            ComponentType::StatusCard => "status_card",
            ComponentType::DataFrame => "dataframe",
            ComponentType::Chart => "chart",
            ComponentType::CodeBlock => "code_block",
            ComponentType::Notification => "notification",
            ComponentType::Badge => "badge",
            ComponentType::IconText => "icon_text",
            ComponentType::StatusIndicator => "status_indicator",
            ComponentType::ProgressBar => "progress_bar",
            ComponentType::ProgressDisplay => "progress_display",
            ComponentType::LogViewer => "log_viewer",
            ComponentType::TaskList => "task_list",
            ComponentType::Button => "button",
            ComponentType::ButtonGroup => "button_group",
            ComponentType::StatusBarUpdate => "status_bar_update",
            ComponentType::TaskTrackerUpdate => "task_tracker_update",
            ComponentType::ChatInputUpdate => "chat_input_update",
        }
    }
}

/// Component lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    Create,
    Update,
    Replace,
    Remove,
}

impl Lifecycle {
    pub fn as_str(self) -> &'static str {
        match self {
            Lifecycle::Create => "create",
            Lifecycle::Update => "update",
            Lifecycle::Replace => "replace",
            Lifecycle::Remove => "remove",
        }
    }
}

/// A rich component: an envelope plus a `data` blob of type-specific fields.
#[derive(Debug, Clone)]
pub struct RichComponent {
    pub id: String,
    pub component_type: ComponentType,
    pub lifecycle: Lifecycle,
    pub data: Map<String, Value>,
    pub children: Vec<String>,
    pub timestamp: String,
    pub visible: bool,
    pub interactive: bool,
}

impl RichComponent {
    fn base(component_type: ComponentType, id: String, data: Map<String, Value>) -> Self {
        Self {
            id,
            component_type,
            lifecycle: Lifecycle::Create,
            data,
            children: Vec::new(),
            timestamp: now_iso(),
            visible: true,
            interactive: false,
        }
    }

    fn auto(component_type: ComponentType, data: Map<String, Value>) -> Self {
        Self::base(component_type, new_uuid(), data)
    }

    /// Serialize to the frontend wire format.
    pub fn serialize_for_frontend(&self) -> Value {
        json!({
            "id": self.id,
            "type": self.component_type.as_str(),
            "lifecycle": self.lifecycle.as_str(),
            "children": self.children,
            "timestamp": self.timestamp,
            "visible": self.visible,
            "interactive": self.interactive,
            "data": Value::Object(self.data.clone()),
        })
    }

    // ---- Constructors for the phase-1 component set ----

    pub fn text(content: impl Into<String>, markdown: bool) -> Self {
        let mut data = Map::new();
        data.insert("content".into(), json!(content.into()));
        data.insert("markdown".into(), json!(markdown));
        Self::auto(ComponentType::Text, data)
    }

    pub fn status_card(
        title: impl Into<String>,
        status: impl Into<String>,
        description: Option<String>,
        icon: Option<String>,
        metadata: Map<String, Value>,
    ) -> Self {
        let mut data = Map::new();
        data.insert("title".into(), json!(title.into()));
        data.insert("status".into(), json!(status.into()));
        data.insert("description".into(), json!(description));
        data.insert("icon".into(), json!(icon));
        data.insert("metadata".into(), Value::Object(metadata));
        Self::auto(ComponentType::StatusCard, data)
    }

    pub fn dataframe(
        records: Vec<Map<String, Value>>,
        columns: Vec<String>,
        title: Option<String>,
    ) -> Self {
        let row_count = records.len();
        let column_count = columns.len();
        let mut data = Map::new();
        data.insert(
            "rows".into(),
            Value::Array(records.into_iter().map(Value::Object).collect()),
        );
        data.insert("columns".into(), json!(columns));
        data.insert("title".into(), json!(title));
        data.insert("row_count".into(), json!(row_count));
        data.insert("column_count".into(), json!(column_count));
        Self::auto(ComponentType::DataFrame, data)
    }

    /// A chart component. `data` is a renderer-ready spec (e.g. a Plotly figure).
    pub fn chart(chart_type: impl Into<String>, data: Value, title: Option<String>) -> Self {
        let mut d = Map::new();
        d.insert("chart_type".into(), json!(chart_type.into()));
        d.insert("data".into(), data);
        d.insert("title".into(), json!(title));
        d.insert("config".into(), json!({}));
        Self::auto(ComponentType::Chart, d)
    }

    pub fn notification(message: impl Into<String>, level: impl Into<String>) -> Self {
        let mut data = Map::new();
        data.insert("message".into(), json!(message.into()));
        data.insert("level".into(), json!(level.into()));
        Self::auto(ComponentType::Notification, data)
    }

    pub fn card(title: impl Into<String>, content: impl Into<String>, markdown: bool) -> Self {
        let mut data = Map::new();
        data.insert("title".into(), json!(title.into()));
        data.insert("content".into(), json!(content.into()));
        data.insert("markdown".into(), json!(markdown));
        Self::auto(ComponentType::Card, data)
    }

    /// A syntax-highlighted code block.
    pub fn code_block(code: impl Into<String>, language: impl Into<String>) -> Self {
        let mut data = Map::new();
        data.insert("content".into(), json!(code.into()));
        data.insert("code_language".into(), json!(language.into()));
        data.insert("markdown".into(), json!(false));
        Self::auto(ComponentType::CodeBlock, data)
    }

    /// A small labelled badge. `variant` ∈ default|primary|success|warning|error|info.
    pub fn badge(text: impl Into<String>, variant: impl Into<String>) -> Self {
        let mut data = Map::new();
        data.insert("text".into(), json!(text.into()));
        data.insert("variant".into(), json!(variant.into()));
        Self::auto(ComponentType::Badge, data)
    }

    /// An icon paired with text.
    pub fn icon_text(icon: impl Into<String>, text: impl Into<String>) -> Self {
        let mut data = Map::new();
        data.insert("icon".into(), json!(icon.into()));
        data.insert("text".into(), json!(text.into()));
        Self::auto(ComponentType::IconText, data)
    }

    /// A status dot + message. `status` ∈ success|warning|error|info|loading.
    pub fn status_indicator(status: impl Into<String>, message: impl Into<String>) -> Self {
        let mut data = Map::new();
        data.insert("status".into(), json!(status.into()));
        data.insert("message".into(), json!(message.into()));
        Self::auto(ComponentType::StatusIndicator, data)
    }

    /// A progress bar. `value` is 0.0–1.0.
    pub fn progress_bar(value: f64, label: Option<String>) -> Self {
        let mut data = Map::new();
        data.insert("value".into(), json!(value.clamp(0.0, 1.0)));
        data.insert("label".into(), json!(label));
        data.insert("show_percentage".into(), json!(true));
        Self::auto(ComponentType::ProgressBar, data)
    }

    /// A labelled progress display with description. `value` is 0.0–1.0.
    pub fn progress_display(
        label: impl Into<String>,
        value: f64,
        description: Option<String>,
    ) -> Self {
        let mut data = Map::new();
        data.insert("label".into(), json!(label.into()));
        data.insert("value".into(), json!(value.clamp(0.0, 1.0)));
        data.insert("description".into(), json!(description));
        data.insert("show_percentage".into(), json!(true));
        Self::auto(ComponentType::ProgressDisplay, data)
    }

    /// A log viewer with structured entries `{timestamp?, level, message}`.
    pub fn log_viewer(title: impl Into<String>, entries: Vec<Value>) -> Self {
        let mut data = Map::new();
        data.insert("title".into(), json!(title.into()));
        data.insert("entries".into(), Value::Array(entries));
        data.insert("show_timestamps".into(), json!(true));
        Self::auto(ComponentType::LogViewer, data)
    }

    /// A list of tasks (each a [`Task`]).
    pub fn task_list(title: impl Into<String>, tasks: Vec<Task>) -> Self {
        let mut data = Map::new();
        data.insert("title".into(), json!(title.into()));
        data.insert(
            "tasks".into(),
            Value::Array(tasks.iter().map(Task::to_value).collect()),
        );
        data.insert("show_progress".into(), json!(true));
        let mut c = Self::auto(ComponentType::TaskList, data);
        c.interactive = true;
        c
    }

    /// A clickable button. `action` is the message sent to chat on click.
    pub fn button(
        label: impl Into<String>,
        action: impl Into<String>,
        variant: impl Into<String>,
    ) -> Self {
        let mut data = Map::new();
        data.insert("label".into(), json!(label.into()));
        data.insert("action".into(), json!(action.into()));
        data.insert("variant".into(), json!(variant.into()));
        let mut c = Self::auto(ComponentType::Button, data);
        c.interactive = true;
        c
    }

    /// A group of buttons, each `{label, action, variant}`.
    pub fn button_group(buttons: Vec<Value>) -> Self {
        let mut data = Map::new();
        data.insert("buttons".into(), Value::Array(buttons));
        data.insert("orientation".into(), json!("horizontal"));
        let mut c = Self::auto(ComponentType::ButtonGroup, data);
        c.interactive = true;
        c
    }

    // ---- Singleton UI-state updates (fixed IDs) ----

    pub fn status_bar(
        status: impl Into<String>,
        message: impl Into<String>,
        detail: Option<String>,
    ) -> Self {
        let mut data = Map::new();
        data.insert("status".into(), json!(status.into()));
        data.insert("message".into(), json!(message.into()));
        data.insert("detail".into(), json!(detail));
        Self::base(ComponentType::StatusBarUpdate, STATUS_BAR_ID.into(), data)
    }

    pub fn chat_input(placeholder: impl Into<String>, disabled: bool) -> Self {
        let mut data = Map::new();
        data.insert("placeholder".into(), json!(placeholder.into()));
        data.insert("disabled".into(), json!(disabled));
        Self::base(ComponentType::ChatInputUpdate, CHAT_INPUT_ID.into(), data)
    }

    pub fn task_tracker(
        operation: TaskOperation,
        task: Option<Task>,
        task_id: Option<String>,
        status: Option<String>,
        detail: Option<String>,
    ) -> Self {
        let mut data = Map::new();
        data.insert("operation".into(), json!(operation.as_str()));
        data.insert("task".into(), task.map_or(Value::Null, |t| t.to_value()));
        data.insert("task_id".into(), json!(task_id));
        data.insert("status".into(), json!(status));
        data.insert("detail".into(), json!(detail));
        Self::base(
            ComponentType::TaskTrackerUpdate,
            TASK_TRACKER_ID.into(),
            data,
        )
    }
}

/// Operation carried by a task-tracker update.
#[derive(Debug, Clone, Copy)]
pub enum TaskOperation {
    AddTask,
    UpdateTask,
    RemoveTask,
    ClearTasks,
}

impl TaskOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskOperation::AddTask => "add_task",
            TaskOperation::UpdateTask => "update_task",
            TaskOperation::RemoveTask => "remove_task",
            TaskOperation::ClearTasks => "clear_tasks",
        }
    }
}

/// A unit of work shown in the task tracker.
#[derive(Debug, Clone)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub status: String,
}

impl Task {
    pub fn new(
        title: impl Into<String>,
        description: Option<String>,
        status: impl Into<String>,
    ) -> Self {
        Self {
            id: new_uuid(),
            title: title.into(),
            description,
            status: status.into(),
        }
    }

    fn to_value(&self) -> Value {
        json!({
            "id": self.id,
            "title": self.title,
            "description": self.description,
            "status": self.status,
        })
    }
}

/// A simple (basic-renderer) component. Phase 1 supports text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimpleComponent {
    #[serde(rename = "type")]
    pub component_type: String,
    /// Type-specific fields (e.g. `text`, or `url`+`alt_text`).
    #[serde(flatten)]
    pub fields: Map<String, Value>,
}

impl SimpleComponent {
    pub fn text(text: impl Into<String>) -> Self {
        let mut fields = Map::new();
        fields.insert("text".into(), json!(text.into()));
        Self {
            component_type: "text".into(),
            fields,
        }
    }

    pub fn image(url: impl Into<String>, alt_text: Option<String>) -> Self {
        let mut fields = Map::new();
        fields.insert("url".into(), json!(url.into()));
        fields.insert("alt_text".into(), json!(alt_text));
        Self {
            component_type: "image".into(),
            fields,
        }
    }

    pub fn link(url: impl Into<String>, text: Option<String>) -> Self {
        let mut fields = Map::new();
        fields.insert("url".into(), json!(url.into()));
        fields.insert("text".into(), json!(text));
        Self {
            component_type: "link".into(),
            fields,
        }
    }

    pub fn serialize_for_frontend(&self) -> Value {
        let mut obj = Map::new();
        obj.insert("type".into(), json!(self.component_type));
        for (k, v) in &self.fields {
            obj.insert(k.clone(), v.clone());
        }
        Value::Object(obj)
    }
}

/// Wrapper yielded by the agent: a rich component plus an optional simple
/// fallback. Mirrors `gauss/core/components.py:UiComponent`.
#[derive(Debug, Clone)]
pub struct UiComponent {
    pub timestamp: String,
    pub rich_component: RichComponent,
    pub simple_component: Option<SimpleComponent>,
}

impl UiComponent {
    pub fn new(rich: RichComponent) -> Self {
        Self {
            timestamp: now_iso(),
            rich_component: rich,
            simple_component: None,
        }
    }

    pub fn with_simple(rich: RichComponent, simple: SimpleComponent) -> Self {
        Self {
            timestamp: now_iso(),
            rich_component: rich,
            simple_component: Some(simple),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rich_component_wire_shape() {
        // The frontend contract: shared fields top-level, component fields under `data`.
        let c = RichComponent::text("hello", true);
        let v = c.serialize_for_frontend();
        assert_eq!(v["type"], "text");
        assert_eq!(v["lifecycle"], "create");
        assert_eq!(v["visible"], true);
        assert_eq!(v["interactive"], false);
        assert!(v["children"].is_array());
        // Component-specific fields live under `data`, not at the top level.
        assert_eq!(v["data"]["content"], "hello");
        assert_eq!(v["data"]["markdown"], true);
        assert!(v.get("content").is_none());
    }

    #[test]
    fn singletons_use_fixed_ids() {
        assert_eq!(
            RichComponent::status_bar("idle", "Ready", None).serialize_for_frontend()["id"],
            STATUS_BAR_ID
        );
        assert_eq!(
            RichComponent::chat_input("Ask...", false).serialize_for_frontend()["id"],
            CHAT_INPUT_ID
        );
        assert_eq!(
            RichComponent::task_tracker(TaskOperation::ClearTasks, None, None, None, None)
                .serialize_for_frontend()["id"],
            TASK_TRACKER_ID
        );
    }

    #[test]
    fn dataframe_records_and_counts() {
        let mut row = Map::new();
        row.insert("name".into(), json!("Acme"));
        let v = RichComponent::dataframe(vec![row], vec!["name".into()], Some("T".into()))
            .serialize_for_frontend();
        assert_eq!(v["type"], "dataframe");
        assert_eq!(v["data"]["row_count"], 1);
        assert_eq!(v["data"]["column_count"], 1);
        assert_eq!(v["data"]["rows"][0]["name"], "Acme");
    }

    #[test]
    fn enum_wire_values() {
        assert_eq!(ComponentType::StatusCard.as_str(), "status_card");
        assert_eq!(ComponentType::DataFrame.as_str(), "dataframe");
        assert_eq!(TaskOperation::AddTask.as_str(), "add_task");
        assert_eq!(Lifecycle::Replace.as_str(), "replace");
    }
}
