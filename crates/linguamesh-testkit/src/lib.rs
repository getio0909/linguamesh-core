#![doc = "无需商业凭据的本地提供商测试服务。"]

use async_stream::stream;
use axum::Router;
use axum::extract::{Json, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use linguamesh_domain::SecretValue;
use serde::Deserialize;
use serde_json::json;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// 管理监听随机回环端口的假提供商。
pub struct FakeProviderServer {
    address: SocketAddr,
    shutdown: CancellationToken,
    task: JoinHandle<()>,
    model_request_counter: Arc<AtomicUsize>,
    chat_request_counter: Arc<AtomicUsize>,
}

impl FakeProviderServer {
    /// 启动兼容 `OpenAI` 模型和流式接口的服务。
    pub async fn start() -> std::io::Result<Self> {
        Self::start_on_port(0).await
    }

    /// 在指定的 IPv4 回环端口启动服务，端口零表示由系统选择。
    pub async fn start_on_port(port: u16) -> std::io::Result<Self> {
        Self::start_with_configuration(port, None, Duration::ZERO).await
    }

    /// 启动在模型响应前等待指定时长的回环服务。
    pub async fn start_with_model_delay(model_delay: Duration) -> std::io::Result<Self> {
        Self::start_with_configuration(0, None, model_delay).await
    }

    /// 启动要求精确 Bearer 凭据的回环服务。
    pub async fn start_requiring_bearer_token(
        expected_token: SecretValue,
    ) -> std::io::Result<Self> {
        Self::start_with_configuration(0, Some(expected_token), Duration::ZERO).await
    }

    async fn start_with_configuration(
        port: u16,
        expected_token: Option<SecretValue>,
        model_delay: Duration,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
        let address = listener.local_addr()?;
        let shutdown = CancellationToken::new();
        let shutdown_signal = shutdown.clone();
        let model_request_counter = Arc::new(AtomicUsize::new(0));
        let chat_request_counter = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/v1/models", get(models))
            .route("/v1/chat/completions", post(chat_completions))
            .with_state(FakeProviderState {
                expected_token: expected_token.map(Arc::new),
                model_delay,
                model_request_counter: Arc::clone(&model_request_counter),
                chat_request_counter: Arc::clone(&chat_request_counter),
            });
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
            model_request_counter,
            chat_request_counter,
        })
    }

    /// 返回包含 `/v1/` 的回环基础地址。
    #[must_use]
    pub fn base_url(&self) -> String {
        format!("http://{}/v1/", self.address)
    }

    /// 返回模型端点已进入处理的请求计数器。
    #[must_use]
    pub fn model_request_counter(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.model_request_counter)
    }

    /// 返回聊天端点已进入处理的请求计数器。
    #[must_use]
    pub fn chat_request_counter(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.chat_request_counter)
    }

    /// 请求干净关闭并等待服务器任务退出。
    pub async fn shutdown(self) {
        self.shutdown.cancel();
        let _ = self.task.await;
    }
}

#[derive(Clone)]
struct FakeProviderState {
    expected_token: Option<Arc<SecretValue>>,
    model_delay: Duration,
    model_request_counter: Arc<AtomicUsize>,
    chat_request_counter: Arc<AtomicUsize>,
}

async fn models(State(state): State<FakeProviderState>, headers: HeaderMap) -> Response {
    state.model_request_counter.fetch_add(1, Ordering::SeqCst);
    if !state.model_delay.is_zero() {
        tokio::time::sleep(state.model_delay).await;
    }
    if !authorized(&state, &headers) {
        return (StatusCode::UNAUTHORIZED, "Authentication failed.").into_response();
    }
    Json(json!({
        "object": "list",
        "data": [
            { "id": "fake-translator", "object": "model" },
            { "id": "fake-slow-translator", "object": "model" }
        ]
    }))
    .into_response()
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

async fn chat_completions(
    State(state): State<FakeProviderState>,
    headers: HeaderMap,
    Json(request): Json<FakeChatRequest>,
) -> Response {
    state.chat_request_counter.fetch_add(1, Ordering::SeqCst);
    if !authorized(&state, &headers) {
        return (StatusCode::UNAUTHORIZED, "Authentication failed.").into_response();
    }
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

fn authorized(state: &FakeProviderState, headers: &HeaderMap) -> bool {
    state.expected_token.as_ref().is_none_or(|expected| {
        headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .is_some_and(|value| value == expected.expose_secret())
    })
}

#[cfg(test)]
mod tests {
    use super::FakeProviderServer;
    use linguamesh_domain::SecretValue;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn server_uses_loopback_and_random_port() {
        let server = FakeProviderServer::start().await.expect("server");
        assert!(server.base_url().starts_with("http://127.0.0.1:"));
        server.shutdown().await;
    }

    #[tokio::test]
    async fn server_accepts_an_explicit_loopback_port() {
        let reservation = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("reservation");
        let port = reservation.local_addr().expect("address").port();
        drop(reservation);

        let server = FakeProviderServer::start_on_port(port)
            .await
            .expect("server");
        assert_eq!(server.base_url(), format!("http://127.0.0.1:{port}/v1/"));
        server.shutdown().await;
    }

    #[tokio::test]
    async fn model_counter_increments_before_configured_delay_finishes() {
        let delay = Duration::from_millis(250);
        let server = FakeProviderServer::start_with_model_delay(delay)
            .await
            .expect("server");
        let counter = server.model_request_counter();
        let endpoint = format!("{}models", server.base_url());
        let started = Instant::now();
        let request = tokio::spawn(async move {
            reqwest::Client::new()
                .get(endpoint)
                .send()
                .await
                .expect("request")
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while counter.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("model handler entry");
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert!(!request.is_finished());
        let response = request.await.expect("request task");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert!(started.elapsed() >= delay);
        server.shutdown().await;
    }

    #[tokio::test]
    async fn server_requires_the_exact_bearer_token() {
        let server =
            FakeProviderServer::start_requiring_bearer_token(SecretValue::new("fake-secret"))
                .await
                .expect("server");
        let endpoint = format!("{}models", server.base_url());
        let client = reqwest::Client::new();
        let unauthorized = client
            .get(&endpoint)
            .send()
            .await
            .expect("unauthorized request");
        assert_eq!(unauthorized.status(), reqwest::StatusCode::UNAUTHORIZED);
        let authorized = client
            .get(endpoint)
            .bearer_auth("fake-secret")
            .send()
            .await
            .expect("authorized request");
        assert_eq!(authorized.status(), reqwest::StatusCode::OK);
        assert_eq!(server.model_request_counter().load(Ordering::SeqCst), 2);
        server.shutdown().await;
    }
}
