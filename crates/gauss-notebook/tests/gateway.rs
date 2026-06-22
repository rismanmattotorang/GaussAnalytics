//! End-to-end gateway tests against a **mock Jupyter Server** — hermetic, no
//! Python required. Proves the N0 contract: start/shut down a kernel over REST,
//! and execute code over the WebSocket channels, collecting normalized outputs.

use axum::extract::Path;
use axum::http::StatusCode;
use axum::routing::{delete, post};
use axum::{Json, Router};
use futures::{SinkExt, StreamExt};
use gauss_notebook::{CellOutput, KernelGateway};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

/// A mock Jupyter REST API: `POST /api/kernels` → an id; `DELETE` → 204.
async fn spawn_rest_mock() -> String {
    let app = Router::new()
        .route(
            "/api/kernels",
            post(|| async { Json(json!({ "id": "kernel-xyz", "name": "python3" })) }),
        )
        .route(
            "/api/kernels/{id}",
            delete(|Path(_id): Path<String>| async { StatusCode::NO_CONTENT }),
        )
        .route(
            "/api/kernels/{id}/interrupt",
            post(|Path(_id): Path<String>| async { StatusCode::NO_CONTENT }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// A mock Jupyter `channels` WebSocket: on receiving an `execute_request`, it
/// replays a realistic `iopub` sequence (busy → stream → result → idle),
/// echoing the request's `msg_id` as the `parent_header` so the client can
/// correlate and terminate.
async fn spawn_ws_mock() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        if let Some(Ok(Message::Text(t))) = ws.next().await {
            let req: Value = serde_json::from_str(&t).unwrap();
            let pid = req["header"]["msg_id"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let seq = [
                json!({"header":{"msg_type":"status"},"parent_header":{"msg_id":pid},"content":{"execution_state":"busy"}}),
                json!({"header":{"msg_type":"stream"},"parent_header":{"msg_id":pid},"content":{"name":"stdout","text":"hello\n"}}),
                json!({"header":{"msg_type":"execute_result"},"parent_header":{"msg_id":pid},"content":{"data":{"text/plain":"'hi'"}}}),
                json!({"header":{"msg_type":"status"},"parent_header":{"msg_id":pid},"content":{"execution_state":"idle"}}),
            ];
            for m in seq {
                ws.send(Message::Text(m.to_string().into())).await.unwrap();
            }
        }
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn start_and_shutdown_kernel_over_rest() {
    let base = spawn_rest_mock().await;
    let gw = KernelGateway::new(base, "tok");
    let id = gw.start_kernel().await.unwrap();
    assert_eq!(id, "kernel-xyz");
    gw.interrupt_kernel(&id).await.unwrap();
    gw.shutdown_kernel(&id).await.unwrap();
}

#[tokio::test]
async fn execute_collects_streamed_outputs_until_idle() {
    let base = spawn_ws_mock().await;
    let gw = KernelGateway::new(base, "");
    let outputs = gw
        .execute_collect("kernel-xyz", "print('hi')")
        .await
        .unwrap();
    assert_eq!(
        outputs,
        vec![
            CellOutput::Stream {
                name: "stdout".into(),
                text: "hello\n".into(),
            },
            CellOutput::Data {
                data: serde_json::from_value(json!({ "text/plain": "'hi'" })).unwrap(),
            },
        ]
    );
}
