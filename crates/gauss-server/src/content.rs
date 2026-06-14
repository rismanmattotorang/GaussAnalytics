//! Analytical content: collections, saved questions (cards), and dashboards,
//! plus content export/import.
//!
//! Content is persisted via the generic [`gauss_db::ContentRepository`] (one
//! table, typed payloads as JSON). Creating/editing content requires the
//! `CreateContent` permission; export is available to any authenticated
//! principal; import is admin-only.

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use chrono::Utc;
use gauss_auth::Permission;
use gauss_core::domain::{
    Card, CardLayout, Collection, Dashboard, DashboardParameter, ParamBinding, ParamKind,
};
use gauss_core::error::CoreError;
use gauss_core::gql::{Filter, Literal, Query};
use gauss_db::ContentRecord;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth;
use crate::error::ApiError;
use crate::state::AppState;
use crate::{execute_query, CompileRequest};

const KIND_CARD: &str = "card";
const KIND_DASHBOARD: &str = "dashboard";
const KIND_COLLECTION: &str = "collection";

async fn require_create(st: &AppState, headers: &HeaderMap) -> Result<auth::CurrentUser, ApiError> {
    let current = auth::authenticate(st, headers).await?;
    current.perms.require(Permission::CreateContent)?;
    Ok(current)
}

fn json_of<T: Serialize>(v: &T) -> Result<String, ApiError> {
    serde_json::to_string(v).map_err(|e| CoreError::Internal(e.to_string()).into())
}

fn parse_all<T: DeserializeOwned>(recs: Vec<ContentRecord>) -> Result<Vec<T>, ApiError> {
    recs.iter()
        .map(|r| serde_json::from_str::<T>(&r.body_json))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| CoreError::Storage(e.to_string()).into())
}

fn parse_one<T: DeserializeOwned>(rec: &ContentRecord) -> Result<T, ApiError> {
    serde_json::from_str::<T>(&rec.body_json).map_err(|e| CoreError::Storage(e.to_string()).into())
}

// --- Collections ---------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateCollectionRequest {
    name: String,
    #[serde(default)]
    parent_id: Option<Uuid>,
}

pub async fn create_collection(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateCollectionRequest>,
) -> Result<Json<Collection>, ApiError> {
    require_create(&st, &headers).await?;
    let coll = Collection {
        id: Uuid::new_v4(),
        name: req.name,
        parent_id: req.parent_id,
    };
    st.store
        .put_content(ContentRecord {
            id: coll.id,
            kind: KIND_COLLECTION.into(),
            collection_id: coll.parent_id,
            name: coll.name.clone(),
            body_json: json_of(&coll)?,
            created_at: Utc::now(),
        })
        .await?;
    Ok(Json(coll))
}

pub async fn list_collections(
    State(st): State<AppState>,
) -> Result<Json<Vec<Collection>>, ApiError> {
    Ok(Json(parse_all(
        st.store.list_content(KIND_COLLECTION).await?,
    )?))
}

// --- Cards (saved questions) --------------------------------------------

#[derive(Deserialize)]
pub struct CreateCardRequest {
    pub name: String,
    pub database_id: Uuid,
    pub query: Query,
    #[serde(default)]
    pub collection_id: Option<Uuid>,
}

pub async fn create_card(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateCardRequest>,
) -> Result<Json<Card>, ApiError> {
    require_create(&st, &headers).await?;
    let card = Card {
        id: Uuid::new_v4(),
        name: req.name,
        database_id: req.database_id,
        query: req.query,
        created_at: Utc::now(),
    };
    st.store
        .put_content(ContentRecord {
            id: card.id,
            kind: KIND_CARD.into(),
            collection_id: req.collection_id,
            name: card.name.clone(),
            body_json: json_of(&card)?,
            created_at: card.created_at,
        })
        .await?;
    Ok(Json(card))
}

pub async fn list_cards(State(st): State<AppState>) -> Result<Json<Vec<Card>>, ApiError> {
    Ok(Json(parse_all(st.store.list_content(KIND_CARD).await?)?))
}

async fn load_card(st: &AppState, id: Uuid) -> Result<Card, ApiError> {
    let rec = st
        .store
        .get_content(id)
        .await?
        .filter(|r| r.kind == KIND_CARD)
        .ok_or_else(|| CoreError::NotFound(format!("card {id}")))?;
    parse_one(&rec)
}

pub async fn get_card(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Card>, ApiError> {
    Ok(Json(load_card(&st, id).await?))
}

pub async fn delete_card(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_create(&st, &headers).await?;
    st.store.delete_content(id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Execute a saved question and return its rows.
pub async fn run_card(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<gauss_drivers::QueryResult>, ApiError> {
    let card = load_card(&st, id).await?;
    let result = execute_query(
        &st,
        &headers,
        &CompileRequest {
            database_id: card.database_id,
            query: card.query,
        },
    )
    .await?;
    Ok(Json(result))
}

// --- Dashboards ----------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateDashboardRequest {
    pub name: String,
    #[serde(default)]
    pub collection_id: Option<Uuid>,
    #[serde(default)]
    pub card_ids: Vec<Uuid>,
    #[serde(default)]
    pub parameters: Vec<DashboardParameter>,
    #[serde(default)]
    pub bindings: Vec<ParamBinding>,
    #[serde(default)]
    pub layout: Vec<CardLayout>,
}

pub async fn create_dashboard(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateDashboardRequest>,
) -> Result<Json<Dashboard>, ApiError> {
    require_create(&st, &headers).await?;
    let dash = Dashboard {
        id: Uuid::new_v4(),
        name: req.name,
        collection_id: req.collection_id,
        card_ids: req.card_ids,
        parameters: req.parameters,
        bindings: req.bindings,
        layout: req.layout,
    };
    persist_dashboard(&st, &dash).await?;
    Ok(Json(dash))
}

async fn persist_dashboard(st: &AppState, dash: &Dashboard) -> Result<(), ApiError> {
    st.store
        .put_content(ContentRecord {
            id: dash.id,
            kind: KIND_DASHBOARD.into(),
            collection_id: dash.collection_id,
            name: dash.name.clone(),
            body_json: json_of(dash)?,
            created_at: Utc::now(),
        })
        .await?;
    Ok(())
}

/// Replace a dashboard's definition (name, cards, filters, layout). Used by the
/// editor to persist drag-and-drop reordering and per-card widths.
pub async fn update_dashboard(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<CreateDashboardRequest>,
) -> Result<Json<Dashboard>, ApiError> {
    require_create(&st, &headers).await?;
    let dash = Dashboard {
        id,
        name: req.name,
        collection_id: req.collection_id,
        card_ids: req.card_ids,
        parameters: req.parameters,
        bindings: req.bindings,
        layout: req.layout,
    };
    persist_dashboard(&st, &dash).await?;
    Ok(Json(dash))
}

pub async fn list_dashboards(State(st): State<AppState>) -> Result<Json<Vec<Dashboard>>, ApiError> {
    Ok(Json(parse_all(
        st.store.list_content(KIND_DASHBOARD).await?,
    )?))
}

pub async fn get_dashboard(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Dashboard>, ApiError> {
    let rec = st
        .store
        .get_content(id)
        .await?
        .filter(|r| r.kind == KIND_DASHBOARD)
        .ok_or_else(|| CoreError::NotFound(format!("dashboard {id}")))?;
    Ok(Json(parse_one(&rec)?))
}

pub async fn delete_dashboard(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_create(&st, &headers).await?;
    st.store.delete_content(id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

// --- Dashboard run (with shared filters) ---------------------------------

#[derive(Deserialize)]
pub struct RunDashboardRequest {
    /// Parameter name → value for the dashboard's shared filters.
    #[serde(default)]
    pub values: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Serialize)]
pub struct DashboardCardResult {
    pub card_id: Uuid,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<gauss_drivers::QueryResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Convert an incoming JSON value into a typed GQL [`Literal`] per the
/// parameter's declared kind. Returns `None` for unknown params / bad values.
fn literal_for(
    params: &[DashboardParameter],
    name: &str,
    v: &serde_json::Value,
) -> Option<Literal> {
    let kind = params.iter().find(|p| p.name == name)?.kind;
    match kind {
        ParamKind::Text => match v {
            serde_json::Value::String(s) => Some(Literal::Text(s.clone())),
            serde_json::Value::Null => None,
            other => Some(Literal::Text(other.to_string())),
        },
        ParamKind::Number => match v {
            serde_json::Value::Number(n) => n.as_f64().map(Literal::Float),
            serde_json::Value::String(s) => s.parse::<f64>().ok().map(Literal::Float),
            _ => None,
        },
    }
}

/// Run every card on a dashboard, injecting the dashboard's shared filter
/// values as **bound GQL filters** into each card's query (parameterized SQL,
/// permission-checked, cached). Per-card failures are reported, not fatal.
pub async fn run_dashboard(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<RunDashboardRequest>,
) -> Result<Json<Vec<DashboardCardResult>>, ApiError> {
    let rec = st
        .store
        .get_content(id)
        .await?
        .filter(|r| r.kind == KIND_DASHBOARD)
        .ok_or_else(|| CoreError::NotFound(format!("dashboard {id}")))?;
    let dash: Dashboard = parse_one(&rec)?;

    let mut out = Vec::with_capacity(dash.card_ids.len());
    for card_id in &dash.card_ids {
        let card = match load_card(&st, *card_id).await {
            Ok(c) => c,
            Err(_) => {
                out.push(DashboardCardResult {
                    card_id: *card_id,
                    name: "(missing card)".into(),
                    result: None,
                    error: Some("card not found".into()),
                });
                continue;
            }
        };

        // Inject dashboard filter values bound to this card.
        let mut query = card.query.clone();
        for b in dash.bindings.iter().filter(|b| &b.card_id == card_id) {
            if let Some(v) = req.values.get(&b.parameter) {
                if let Some(lit) = literal_for(&dash.parameters, &b.parameter, v) {
                    query.filters.push(Filter::Compare {
                        field: b.field.clone(),
                        op: b.op,
                        value: lit,
                    });
                }
            }
        }

        match execute_query(
            &st,
            &headers,
            &CompileRequest {
                database_id: card.database_id,
                query,
            },
        )
        .await
        {
            Ok(result) => out.push(DashboardCardResult {
                card_id: *card_id,
                name: card.name,
                result: Some(result),
                error: None,
            }),
            Err(e) => out.push(DashboardCardResult {
                card_id: *card_id,
                name: card.name,
                result: None,
                error: Some(e.0.to_string()),
            }),
        }
    }
    Ok(Json(out))
}

// --- Export / import -----------------------------------------------------

/// A portable bundle of all analytical content.
#[derive(Serialize, Deserialize)]
pub struct ContentBundle {
    pub collections: Vec<Collection>,
    pub cards: Vec<Card>,
    pub dashboards: Vec<Dashboard>,
}

/// Export all content as a portable bundle (any authenticated principal).
pub async fn export_content(
    State(st): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ContentBundle>, ApiError> {
    auth::authenticate(&st, &headers).await?;
    Ok(Json(ContentBundle {
        collections: parse_all(st.store.list_content(KIND_COLLECTION).await?)?,
        cards: parse_all(st.store.list_content(KIND_CARD).await?)?,
        dashboards: parse_all(st.store.list_content(KIND_DASHBOARD).await?)?,
    }))
}

#[derive(Serialize)]
pub struct ImportSummary {
    collections: usize,
    cards: usize,
    dashboards: usize,
}

/// Import a content bundle, upserting by id (admin only).
pub async fn import_content(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(bundle): Json<ContentBundle>,
) -> Result<Json<ImportSummary>, ApiError> {
    let current = auth::authenticate(&st, &headers).await?;
    current.perms.require(Permission::ManageSettings)?;

    for c in &bundle.collections {
        st.store
            .put_content(ContentRecord {
                id: c.id,
                kind: KIND_COLLECTION.into(),
                collection_id: c.parent_id,
                name: c.name.clone(),
                body_json: json_of(c)?,
                created_at: Utc::now(),
            })
            .await?;
    }
    for c in &bundle.cards {
        st.store
            .put_content(ContentRecord {
                id: c.id,
                kind: KIND_CARD.into(),
                collection_id: None,
                name: c.name.clone(),
                body_json: json_of(c)?,
                created_at: c.created_at,
            })
            .await?;
    }
    for d in &bundle.dashboards {
        st.store
            .put_content(ContentRecord {
                id: d.id,
                kind: KIND_DASHBOARD.into(),
                collection_id: d.collection_id,
                name: d.name.clone(),
                body_json: json_of(d)?,
                created_at: Utc::now(),
            })
            .await?;
    }

    Ok(Json(ImportSummary {
        collections: bundle.collections.len(),
        cards: bundle.cards.len(),
        dashboards: bundle.dashboards.len(),
    }))
}
