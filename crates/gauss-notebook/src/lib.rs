//! Kernel gateway for GaussAnalytics' embedded notebooks (initiative N0 spike).
//!
//! Per [`docs/NOTEBOOKS_PLAN.md`], notebook code runs on the **user's local
//! Jupyter Server**, not inside the Rust process. This crate is the client that
//! drives it: start/stop kernels over the Jupyter Server **REST** API and run
//! code over its **WebSocket** channels, normalizing the kernel's `iopub`
//! messages into a small [`CellOutput`] stream the rest of GaussAnalytics can
//! render. Using Jupyter Server's HTTP/WS API (rather than the raw ZeroMQ wire
//! protocol) keeps this small and robust.
//!
//! N0 proves the contract end-to-end against a mock Jupyter (see tests); the
//! notebook document model, server routes, and UI land in later phases. The
//! whole feature sits behind `GAUSS_JUPYTER_ENABLED` (off by default), so it
//! never affects a default build.

#![forbid(unsafe_code)]

pub mod dag;
pub mod nbformat;

use futures::{SinkExt, StreamExt};
use gauss_core::error::{CoreError, CoreResult};
use serde::Serialize;
use serde_json::{json, Map, Value};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

/// One normalized output produced by executing a cell, in arrival order.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CellOutput {
    /// `stdout`/`stderr` text (`iopub` `stream`).
    Stream { name: String, text: String },
    /// A rich MIME bundle (`execute_result` / `display_data`): `{mime: value}`.
    Data { data: Map<String, Value> },
    /// An execution error with its traceback (`iopub` `error`).
    Error {
        ename: String,
        evalue: String,
        traceback: Vec<String>,
    },
}

/// A classified inbound kernel message (only the parts N0 needs).
#[derive(Debug, Clone, PartialEq)]
enum KernelMessage {
    /// `execution_state`: `busy` / `idle` / `starting`.
    Status(String),
    Output(CellOutput),
    /// The shell `execute_reply` that terminates a request.
    ExecuteReply,
    Other,
}

fn integ<E: std::fmt::Display>(e: E) -> CoreError {
    CoreError::Integration(e.to_string())
}

/// Build a Jupyter v5 message envelope for the Jupyter Server WebSocket
/// (which carries each ZeroMQ multipart as one JSON object).
fn message(session: &str, channel: &str, msg_type: &str, content: Value) -> (String, Value) {
    let msg_id = Uuid::new_v4().to_string();
    let envelope = json!({
        "header": {
            "msg_id": msg_id,
            "session": session,
            "username": "gauss",
            "msg_type": msg_type,
            "version": "5.3",
        },
        "parent_header": {},
        "metadata": {},
        "content": content,
        "channel": channel,
        "buffers": [],
    });
    (msg_id, envelope)
}

/// An `execute_request` for `code` on the `shell` channel. Returns the message's
/// `msg_id` (so replies can be correlated) and the envelope to send.
fn execute_request(session: &str, code: &str) -> (String, Value) {
    message(
        session,
        "shell",
        "execute_request",
        json!({
            "code": code,
            "silent": false,
            "store_history": true,
            "user_expressions": {},
            "allow_stdin": false,
            "stop_on_error": true,
        }),
    )
}

fn str_at<'a>(v: &'a Value, ptr: &str) -> &'a str {
    v.pointer(ptr).and_then(Value::as_str).unwrap_or_default()
}

/// Classify an inbound message and, when it is for `parent_msg_id`, normalize
/// any output. Messages for other requests are ignored as [`KernelMessage::Other`].
fn classify(msg: &Value, parent_msg_id: &str) -> KernelMessage {
    let msg_type = str_at(msg, "/header/msg_type");
    let parent = str_at(msg, "/parent_header/msg_id");
    if !parent.is_empty() && parent != parent_msg_id {
        return KernelMessage::Other;
    }
    match msg_type {
        "status" => KernelMessage::Status(str_at(msg, "/content/execution_state").to_string()),
        "stream" => KernelMessage::Output(CellOutput::Stream {
            name: str_at(msg, "/content/name").to_string(),
            text: str_at(msg, "/content/text").to_string(),
        }),
        "execute_result" | "display_data" => {
            let data = msg
                .pointer("/content/data")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            KernelMessage::Output(CellOutput::Data { data })
        }
        "error" => {
            let traceback = msg
                .pointer("/content/traceback")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            KernelMessage::Output(CellOutput::Error {
                ename: str_at(msg, "/content/ename").to_string(),
                evalue: str_at(msg, "/content/evalue").to_string(),
                traceback,
            })
        }
        "execute_reply" => KernelMessage::ExecuteReply,
        _ => KernelMessage::Other,
    }
}

/// A client for a (user-local) Jupyter Server.
pub struct KernelGateway {
    client: reqwest::Client,
    /// Base HTTP URL, e.g. `http://127.0.0.1:8888` (no trailing slash).
    base_url: String,
    /// Jupyter Server token (may be empty for token-less local servers).
    token: String,
}

impl KernelGateway {
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            token: token.into(),
        }
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.token.is_empty() {
            req
        } else {
            req.header("Authorization", format!("token {}", self.token))
        }
    }

    /// Start a Python kernel; returns its id (`POST /api/kernels`).
    pub async fn start_kernel(&self) -> CoreResult<String> {
        let url = format!("{}/api/kernels", self.base_url);
        let resp = self
            .auth(self.client.post(&url).json(&json!({ "name": "python3" })))
            .send()
            .await
            .map_err(integ)?
            .error_for_status()
            .map_err(integ)?;
        let body: Value = resp.json().await.map_err(integ)?;
        body.get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| integ("kernel start response missing id"))
    }

    /// Shut down a kernel (`DELETE /api/kernels/{id}`).
    pub async fn shutdown_kernel(&self, kernel_id: &str) -> CoreResult<()> {
        let url = format!("{}/api/kernels/{kernel_id}", self.base_url);
        self.auth(self.client.delete(&url))
            .send()
            .await
            .map_err(integ)?
            .error_for_status()
            .map_err(integ)?;
        Ok(())
    }

    /// Interrupt a running kernel (`POST /api/kernels/{id}/interrupt`). Used to
    /// stop a long-running or runaway cell without restarting the kernel.
    pub async fn interrupt_kernel(&self, kernel_id: &str) -> CoreResult<()> {
        let url = format!("{}/api/kernels/{kernel_id}/interrupt", self.base_url);
        self.auth(self.client.post(&url))
            .send()
            .await
            .map_err(integ)?
            .error_for_status()
            .map_err(integ)?;
        Ok(())
    }

    /// The `channels` WebSocket URL for a kernel (http→ws, https→wss).
    fn channels_url(&self, kernel_id: &str) -> String {
        let ws = if let Some(rest) = self.base_url.strip_prefix("https://") {
            format!("wss://{rest}")
        } else if let Some(rest) = self.base_url.strip_prefix("http://") {
            format!("ws://{rest}")
        } else {
            self.base_url.clone()
        };
        let mut url = format!("{ws}/api/kernels/{kernel_id}/channels");
        if !self.token.is_empty() {
            url.push_str(&format!("?token={}", self.token));
        }
        url
    }

    /// Execute `code` on `kernel_id` and collect its outputs in order, returning
    /// once the kernel reports `idle` for this request (or sends `execute_reply`).
    ///
    /// N0 collects outputs; the streaming-to-browser path is added with the
    /// notebook server routes in N1.
    pub async fn execute_collect(
        &self,
        kernel_id: &str,
        code: &str,
    ) -> CoreResult<Vec<CellOutput>> {
        let url = self.channels_url(kernel_id);
        let (mut ws, _resp) = tokio_tungstenite::connect_async(&url)
            .await
            .map_err(integ)?;

        let session = Uuid::new_v4().to_string();
        let (msg_id, envelope) = execute_request(&session, code);
        ws.send(Message::Text(envelope.to_string().into()))
            .await
            .map_err(integ)?;

        let mut outputs = Vec::new();
        let mut saw_reply = false;
        while let Some(frame) = ws.next().await {
            let frame = frame.map_err(integ)?;
            let text = match frame {
                Message::Text(t) => t.to_string(),
                Message::Binary(b) => String::from_utf8_lossy(&b).into_owned(),
                Message::Close(_) => break,
                _ => continue,
            };
            let Ok(msg) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            match classify(&msg, &msg_id) {
                KernelMessage::Output(o) => outputs.push(o),
                KernelMessage::ExecuteReply => saw_reply = true,
                // `idle` after our request means execution is complete.
                KernelMessage::Status(state) if state == "idle" => {
                    let _ = ws.close(None).await;
                    break;
                }
                _ => {}
            }
        }
        let _ = saw_reply;
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execute_request_has_correct_shape() {
        let (msg_id, env) = execute_request("sess-1", "print('hi')");
        assert_eq!(str_at(&env, "/channel"), "shell");
        assert_eq!(str_at(&env, "/header/msg_type"), "execute_request");
        assert_eq!(str_at(&env, "/header/session"), "sess-1");
        assert_eq!(str_at(&env, "/content/code"), "print('hi')");
        assert_eq!(str_at(&env, "/header/msg_id"), msg_id);
        assert_eq!(env.pointer("/content/allow_stdin"), Some(&json!(false)));
    }

    #[test]
    fn classify_normalizes_iopub_messages() {
        let parent = "p1";
        let mk = |ty: &str, content: Value| json!({"header":{"msg_type":ty},"parent_header":{"msg_id":parent},"content":content});
        assert_eq!(
            classify(&mk("status", json!({"execution_state":"idle"})), parent),
            KernelMessage::Status("idle".into())
        );
        assert_eq!(
            classify(
                &mk("stream", json!({"name":"stdout","text":"hi\n"})),
                parent
            ),
            KernelMessage::Output(CellOutput::Stream {
                name: "stdout".into(),
                text: "hi\n".into()
            })
        );
        match classify(
            &mk("execute_result", json!({"data":{"text/plain":"3"}})),
            parent,
        ) {
            KernelMessage::Output(CellOutput::Data { data }) => {
                assert_eq!(data["text/plain"], json!("3"));
            }
            other => panic!("expected data, got {other:?}"),
        }
        match classify(
            &mk(
                "error",
                json!({"ename":"ValueError","evalue":"bad","traceback":["a","b"]}),
            ),
            parent,
        ) {
            KernelMessage::Output(CellOutput::Error {
                ename, traceback, ..
            }) => {
                assert_eq!(ename, "ValueError");
                assert_eq!(traceback, vec!["a".to_string(), "b".to_string()]);
            }
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn messages_for_other_requests_are_ignored() {
        let msg = json!({
            "header":{"msg_type":"stream"},
            "parent_header":{"msg_id":"someone-else"},
            "content":{"name":"stdout","text":"x"}
        });
        assert_eq!(classify(&msg, "mine"), KernelMessage::Other);
    }

    #[test]
    fn channels_url_upgrades_scheme_and_adds_token() {
        let gw = KernelGateway::new("http://127.0.0.1:8888", "secret");
        assert_eq!(
            gw.channels_url("k1"),
            "ws://127.0.0.1:8888/api/kernels/k1/channels?token=secret"
        );
        let gw = KernelGateway::new("https://host/", "");
        assert_eq!(gw.channels_url("k2"), "wss://host/api/kernels/k2/channels");
    }
}
