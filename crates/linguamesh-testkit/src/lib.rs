#![doc = "无需商业凭据的本地提供商测试服务。"]

use async_stream::stream;
use axum::Router;
use axum::extract::Json;
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::json;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// 管理监听随机回环端口的假提供商。
pub struct FakeProviderServer {
    address: SocketAddr,
    shutdown: CancellationToken,
    task: JoinHandle<()>,
}

impl FakeProviderServer {
    /// 启动兼容 `OpenAI` 模型和流式接口的服务。
    pub async fn start() -> std::io::Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let address = listener.local_addr()?;
        let shutdown = CancellationToken::new();
        let shutdown_signal = shutdown.clone();
        let app = Router::new()
            .route("/v1/models", get(models))
            .route("/v1/chat/completions", post(chat_completions));
        let task = tokio::spawn(async move {
            let result = axum::serve(listener, app)
                .with_graceful_shutdown(shutdown_signal.cancelled_owned())
                .await;
            if let Err(error) = result {
                eprintln!("Fake provider stopped unexpectedly: {error}");
            }
        });
        Ok(Self {
            address,
            shutdown,
            task,
        })
    }

    /// 返回包含 `/v1/` 的回环基础地址。
    #[must_use]
    pub fn base_url(&self) -> String {
        format!("http://{}/v1/", self.address)
    }

    /// 请求干净关闭并等待服务器任务退出。
    pub async fn shutdown(self) {
        self.shutdown.cancel();
        let _ = self.task.await;
    }
}

async fn models() -> impl IntoResponse {
    Json(json!({
        "object": "list",
        "data": [
            { "id": "fake-translator", "object": "model" },
            { "id": "fake-slow-translator", "object": "model" }
        ]
    }))
}

#[derive(Deserialize)]
struct FakeChatRequest {
    model: String,
    messages: Vec<FakeMessage>,
}

#[derive(Deserialize)]
struct FakeMessage {
    content: String,
}

async fn chat_completions(Json(request): Json<FakeChatRequest>) -> Response {
    let source = request
        .messages
        .last()
        .map(|message| message.content.as_str())
        .unwrap_or_default();
    if source.contains("[auth-error]") {
        return (StatusCode::UNAUTHORIZED, "Authentication failed.").into_response();
    }
    let malformed = source.contains("[malformed]");
    let disconnect = source.contains("[disconnect]");
    let delay = if request.model == "fake-slow-translator" {
        Duration::from_millis(250)
    } else {
        Duration::from_millis(35)
    };
    let output = stream! {
        let fragments = ["你好", "，", "LinguaMesh", "！"];
        for fragment in fragments {
            tokio::time::sleep(delay).await;
            let data = if malformed {
                "{not-json".to_owned()
            } else {
                json!({
                    "choices": [{ "delta": { "content": fragment } }]
                })
                .to_string()
            };
            yield Ok::<Event, Infallible>(Event::default().data(data));
            if malformed {
                return;
            }
        }
        if !disconnect {
            yield Ok::<Event, Infallible>(Event::default().data("[DONE]"));
        }
    };
    Sse::new(output)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(5)))
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::FakeProviderServer;

    #[tokio::test]
    async fn server_uses_loopback_and_random_port() {
        let server = FakeProviderServer::start().await.expect("server");
        assert!(server.base_url().starts_with("http://127.0.0.1:"));
        server.shutdown().await;
    }
}
