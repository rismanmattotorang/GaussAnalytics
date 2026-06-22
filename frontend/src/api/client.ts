// Typed client for the GaussAnalytics HTTP API.
//
// These types mirror the Rust server's JSON contract (see crates/gauss-server
// and crates/gauss-core). Keeping the contract in one place is what lets the
// web UI and the Rust backend evolve independently.

export interface Health {
  status: string;
  name: string;
  version: string;
}

export type DataSourceKind =
  | "postgres"
  | "mysql"
  | "sqlite"
  | "oracle"
  | "bigquery"
  | "snowflake"
  | "clickhouse"
  | "generic";
export type FieldType = "integer" | "float" | "text" | "boolean" | "datetime" | "unknown";
export type SemanticType =
  | "category"
  | "quantity"
  | "temporal"
  | "text"
  | "key"
  | "unknown";

export interface Database {
  id: string;
  name: string;
  kind: DataSourceKind;
  is_synced: boolean;
  connection_uri?: string | null;
  created_at: string;
}

/** Result of probing a candidate data-source connection. */
export interface TestConnectionResult {
  ok: boolean;
  table_count: number;
}

/** Result of syncing a data source's schema. */
export interface SyncResult {
  database_id: string;
  tables: Array<{ name: string; columns: number }>;
}

/** Current AI/NL2SQL configuration (secrets redacted). */
export interface AiSettings {
  enabled: boolean;
  provider: string;
  model: string;
  base_url: string;
  has_api_key: boolean;
  supported_providers: string[];
}

export interface Field {
  id: string;
  name: string;
  field_type: FieldType;
  semantic_type?: SemanticType | null;
}

export interface Table {
  id: string;
  database_id: string;
  name: string;
  fields: Field[];
}

// --- GQL: the structured query the UI builds ----------------------------

export type Literal =
  | { kind: "int"; value: number }
  | { kind: "float"; value: number }
  | { kind: "text"; value: string }
  | { kind: "bool"; value: boolean }
  | { kind: "null" };

export type CompareOp = "eq" | "ne" | "lt" | "le" | "gt" | "ge";

export type Filter =
  | { type: "compare"; args: { field: string; op: CompareOp; value: Literal } }
  | { type: "in"; args: { field: string; values: Literal[] } }
  | { type: "is_null"; args: { field: string } }
  | { type: "is_not_null"; args: { field: string } }
  | { type: "and"; args: Filter[] }
  | { type: "or"; args: Filter[] }
  | { type: "not"; args: Filter };

export type AggFunc = "count" | "count_distinct" | "sum" | "avg" | "min" | "max";

export interface Aggregation {
  func: AggFunc;
  field?: string | null;
  alias?: string | null;
}

export interface OrderBy {
  field: string;
  direction?: "asc" | "desc";
}

export interface Query {
  source_table: string;
  fields?: string[];
  filters?: Filter[];
  aggregations?: Aggregation[];
  breakouts?: string[];
  order_by?: OrderBy[];
  limit?: number | null;
}

export type SqlParam =
  | { type: "int"; value: number }
  | { type: "float"; value: number }
  | { type: "text"; value: string }
  | { type: "bool"; value: boolean }
  | { type: "null" };

export interface CompiledQuery {
  sql: string;
  params: SqlParam[];
}

export interface QueryResult {
  columns: string[];
  rows: unknown[][];
}

export interface GuardedQuery {
  sql: string;
  explanation?: string | null;
  confidence?: number | null;
}

export interface Session {
  token: string;
  expires_at: string;
}

export interface Card {
  id: string;
  name: string;
  database_id: string;
  query: Query;
  created_at: string;
}

export interface RlsPolicy {
  id: string;
  database_id: string;
  table: string;
  column: string;
  op: CompareOp;
  value: Literal;
}

export type ParamKind = "text" | "number";

export interface DashboardParameter {
  name: string;
  kind: ParamKind;
}

export interface ParamBinding {
  parameter: string;
  card_id: string;
  field: string;
  op?: CompareOp;
}

export interface CardLayout {
  card_id: string;
  w: number;
}

export interface DashboardTab {
  name: string;
  card_ids: string[];
}

/** A free-form Markdown panel on a dashboard (titles, notes, links). */
export interface DashboardTextCard {
  id: string;
  markdown: string;
  w?: number;
}

export interface Dashboard {
  id: string;
  name: string;
  collection_id?: string | null;
  card_ids: string[];
  parameters?: DashboardParameter[];
  bindings?: ParamBinding[];
  layout?: CardLayout[];
  links?: string[];
  tabs?: DashboardTab[];
  text_cards?: DashboardTextCard[];
}

export interface DashboardCardResult {
  card_id: string;
  name: string;
  result?: QueryResult;
  error?: string;
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const headers: Record<string, string> = {
    "content-type": "application/json",
    ...(init?.headers as Record<string, string> | undefined),
  };
  const res = await fetch(`/api${path}`, { ...init, headers });
  if (!res.ok) {
    const body = await res.json().catch(() => ({ error: res.statusText }));
    throw new Error(body.error ?? `request failed: ${res.status}`);
  }
  return res.json() as Promise<T>;
}

/// Authenticated request: merges a bearer token into the headers.
function authed<T>(path: string, method: string, token: string, body?: unknown): Promise<T> {
  return request<T>(path, {
    method,
    headers: { Authorization: `Bearer ${token}` },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
}

export const api = {
  health: () => request<Health>("/health"),
  databases: () => request<Database[]>("/databases"),
  tables: (databaseId: string) => request<Table[]>(`/databases/${databaseId}/tables`),
  createDatabase: (
    body: { name: string; kind: DataSourceKind; connection_uri?: string },
    token: string,
  ) => authed<Database>("/databases", "POST", token, body),
  testConnection: (
    body: { kind: DataSourceKind; connection_uri: string },
    token: string,
  ) => authed<TestConnectionResult>("/databases/test", "POST", token, body),
  syncDatabase: (id: string, token: string) =>
    authed<SyncResult>(`/databases/${id}/sync`, "POST", token),
  deleteDatabase: (id: string, token: string) =>
    authed<{ deleted: string }>(`/databases/${id}`, "DELETE", token),
  aiSettings: (token: string) => authed<AiSettings>("/settings/ai", "GET", token),
  updateAiSettings: (
    body: {
      enabled?: boolean;
      provider?: string;
      model?: string;
      base_url?: string;
      api_key?: string;
    },
    token: string,
  ) => authed<AiSettings>("/settings/ai", "PUT", token, body),
  run: (database_id: string, query: Query) =>
    request<QueryResult>("/dataset/run", {
      method: "POST",
      body: JSON.stringify({ database_id, query }),
    }),
  compile: (database_id: string, query: Query) =>
    request<CompiledQuery>("/dataset/compile", {
      method: "POST",
      body: JSON.stringify({ database_id, query }),
    }),
  native: (database_id: string, sql: string, params: unknown[] = []) =>
    request<QueryResult>("/dataset/native", {
      method: "POST",
      body: JSON.stringify({ database_id, sql, params }),
    }),
  nl2sql: (database_id: string, prompt: string) =>
    request<GuardedQuery>("/nl2sql", {
      method: "POST",
      body: JSON.stringify({ database_id, prompt }),
    }),
  login: (email: string, password: string) =>
    request<Session>("/auth/login", {
      method: "POST",
      body: JSON.stringify({ email, password }),
    }),
  cards: () => request<Card[]>("/cards"),
  createCard: (
    body: { name: string; database_id: string; query: Query },
    token: string,
  ) => authed<Card>("/cards", "POST", token, body),
  runCard: (id: string) => request<QueryResult>(`/cards/${id}/run`, { method: "POST" }),
  dashboards: () => request<Dashboard[]>("/dashboards"),
  createDashboard: (
    body: {
      name: string;
      card_ids: string[];
      parameters?: DashboardParameter[];
      bindings?: ParamBinding[];
      links?: string[];
      tabs?: DashboardTab[];
      text_cards?: DashboardTextCard[];
    },
    token: string,
  ) => authed<Dashboard>("/dashboards", "POST", token, body),
  runDashboard: (id: string, values: Record<string, unknown>) =>
    request<DashboardCardResult[]>(`/dashboards/${id}/run`, {
      method: "POST",
      body: JSON.stringify({ values }),
    }),
  updateDashboard: (
    id: string,
    body: {
      name: string;
      card_ids: string[];
      parameters?: DashboardParameter[];
      bindings?: ParamBinding[];
      layout?: CardLayout[];
      links?: string[];
      tabs?: DashboardTab[];
      text_cards?: DashboardTextCard[];
    },
    token: string,
  ) => authed<Dashboard>(`/dashboards/${id}`, "PUT", token, body),
  metrics: () => request<Card[]>("/metrics"),
  createMetric: (
    body: { name: string; database_id: string; query: Query },
    token: string,
  ) => authed<Card>("/metrics", "POST", token, body),
  runMetric: (id: string) => request<QueryResult>(`/metrics/${id}/run`, { method: "POST" }),
  rlsPolicies: (token: string) => authed<RlsPolicy[]>("/rls", "GET", token),
  createRls: (
    body: {
      database_id: string;
      table: string;
      column: string;
      op?: CompareOp;
      value: Literal;
    },
    token: string,
  ) => authed<RlsPolicy>("/rls", "POST", token, body),
  exportContent: (token: string) =>
    authed<{ collections: unknown[]; cards: Card[]; dashboards: Dashboard[] }>(
      "/export",
      "GET",
      token,
    ),
};
