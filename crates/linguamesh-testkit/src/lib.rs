#![doc = "无需商业凭据的本地提供商测试服务。"]

use async_stream::stream;
use axum::Router;
use axum::body::{Body, Bytes};
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
        Self::start_with_configuration(
            0,
            None,
            Duration::ZERO,
            FakeProviderFlavor::Standard,
            None,
            None,
        )
        .await
    }

    /// 在指定的 IPv4 回环端口启动服务，端口零表示由系统选择。
    pub async fn start_on_port(port: u16) -> std::io::Result<Self> {
        Self::start_with_configuration(
            port,
            None,
            Duration::ZERO,
            FakeProviderFlavor::Standard,
            None,
            None,
        )
        .await
    }

    /// 启动在模型响应前等待指定时长的回环服务。
    pub async fn start_with_model_delay(model_delay: Duration) -> std::io::Result<Self> {
        Self::start_with_configuration(
            0,
            None,
            model_delay,
            FakeProviderFlavor::Standard,
            None,
            None,
        )
        .await
    }

    /// 启动要求精确 Bearer 凭据的回环服务。
    pub async fn start_requiring_bearer_token(
        expected_token: SecretValue,
    ) -> std::io::Result<Self> {
        Self::start_with_configuration(
            0,
            Some(expected_token),
            Duration::ZERO,
            FakeProviderFlavor::Standard,
            None,
            None,
        )
        .await
    }

    /// 启动返回 Ollama 模型标识的 `OpenAI` 兼容回环服务。
    pub async fn start_ollama_compatible() -> std::io::Result<Self> {
        Self::start_with_configuration(
            0,
            None,
            Duration::ZERO,
            FakeProviderFlavor::Ollama,
            None,
            None,
        )
        .await
    }

    /// 启动原生 Ollama `/api` 模型和 NDJSON 聊天接口的回环服务。
    pub async fn start_ollama_native() -> std::io::Result<Self> {
        Self::start_with_configuration(
            0,
            None,
            Duration::ZERO,
            FakeProviderFlavor::OllamaNative,
            None,
            None,
        )
        .await
    }

    /// 启动 Gemini Generate Content 模型和 SSE 接口的回环服务。
    pub async fn start_gemini() -> std::io::Result<Self> {
        Self::start_with_configuration(
            0,
            None,
            Duration::ZERO,
            FakeProviderFlavor::Gemini,
            None,
            None,
        )
        .await
    }

    /// 启动要求 `api-key` 请求头的 Azure `OpenAI` 回环服务。
    pub async fn start_azure() -> std::io::Result<Self> {
        Self::start_with_configuration(
            0,
            Some(SecretValue::new("azure-test-key")),
            Duration::ZERO,
            FakeProviderFlavor::Azure,
            None,
            None,
        )
        .await
    }

    /// 启动要求 Bearer 凭据并返回 typed SSE 事件的 Responses API 回环服务。
    pub async fn start_responses() -> std::io::Result<Self> {
        Self::start_with_configuration(
            0,
            Some(SecretValue::new("responses-test-key")),
            Duration::ZERO,
            FakeProviderFlavor::Responses,
            None,
            None,
        )
        .await
    }

    /// 启动要求 `OpenAI` 项目请求头并返回兼容 Chat 流的回环服务。
    pub async fn start_requiring_openai_project(
        project: impl Into<String>,
    ) -> std::io::Result<Self> {
        Self::start_with_configuration(
            0,
            None,
            Duration::ZERO,
            FakeProviderFlavor::Standard,
            Some(project.into()),
            None,
        )
        .await
    }

    /// 启动同时要求 `OpenAI` 项目请求头和一个安全自定义请求头的回环服务。
    pub async fn start_requiring_openai_project_and_custom_header(
        project: impl Into<String>,
        custom_header: (String, String),
    ) -> std::io::Result<Self> {
        Self::start_with_configuration(
            0,
            None,
            Duration::ZERO,
            FakeProviderFlavor::Standard,
            Some(project.into()),
            Some(custom_header),
        )
        .await
    }

    /// 启动要求 `OpenAI` 项目请求头并返回 Responses typed-SSE 的回环服务。
    pub async fn start_responses_requiring_openai_project(
        project: impl Into<String>,
    ) -> std::io::Result<Self> {
        Self::start_with_configuration(
            0,
            Some(SecretValue::new("responses-test-key")),
            Duration::ZERO,
            FakeProviderFlavor::Responses,
            Some(project.into()),
            None,
        )
        .await
    }

    /// 启动要求 Azure 自定义请求头和 `api-key` 的回环服务。
    pub async fn start_azure_requiring_custom_header() -> std::io::Result<Self> {
        Self::start_with_configuration(
            0,
            Some(SecretValue::new("azure-test-key")),
            Duration::ZERO,
            FakeProviderFlavor::Azure,
            None,
            Some(("X-Trace-Mode".to_owned(), "azure".to_owned())),
        )
        .await
    }

    async fn start_with_configuration(
        port: u16,
        expected_token: Option<SecretValue>,
        model_delay: Duration,
        flavor: FakeProviderFlavor,
        expected_project: Option<String>,
        expected_custom_header: Option<(String, String)>,
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
            .route(
                "/openai/deployments/fake-deployment/chat/completions",
                post(azure_chat_completions),
            )
            .route("/v1/responses", post(responses))
            .route("/api/tags", get(ollama_tags))
            .route("/api/chat", post(ollama_chat))
            .route("/v1beta/models", get(gemini_models))
            .route(
                "/v1beta/models/gemini-2.0-flash:streamGenerateContent",
                post(gemini_stream),
            )
            .with_state(FakeProviderState {
                expected_token: expected_token.map(Arc::new),
                expected_project,
                expected_custom_header,
                model_delay,
                flavor,
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

    /// 返回包含 `/api/` 的原生 Ollama 回环基础地址。
    #[must_use]
    pub fn ollama_base_url(&self) -> String {
        format!("http://{}/api/", self.address)
    }

    /// 返回包含 `/v1beta/` 的 Gemini 回环基础地址。
    #[must_use]
    pub fn gemini_base_url(&self) -> String {
        format!("http://{}/v1beta/", self.address)
    }

    /// 返回 Azure `OpenAI` 资源根回环基础地址。
    #[must_use]
    pub fn azure_base_url(&self) -> String {
        format!("http://{}/", self.address)
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
    expected_project: Option<String>,
    expected_custom_header: Option<(String, String)>,
    model_delay: Duration,
    flavor: FakeProviderFlavor,
    model_request_counter: Arc<AtomicUsize>,
    chat_request_counter: Arc<AtomicUsize>,
}

#[derive(Clone, Copy)]
enum FakeProviderFlavor {
    Standard,
    Ollama,
    OllamaNative,
    Gemini,
    Azure,
    Responses,
}

// 返回最小的 Gemini 模型发现响应，确保测试只依赖本地回环服务。
async fn gemini_models(State(state): State<FakeProviderState>, headers: HeaderMap) -> Response {
    state.model_request_counter.fetch_add(1, Ordering::SeqCst);
    if !authorized(&state, &headers) {
        return (StatusCode::UNAUTHORIZED, "Authentication failed.").into_response();
    }
    Json(json!({
        "models": [
            {
                "name": "models/gemini-2.0-flash",
                "displayName": "Gemini 2.0 Flash",
                "supportedGenerationMethods": ["generateContent"]
            },
            {
                "name": "models/text-embedding-004",
                "displayName": "Text Embedding",
                "supportedGenerationMethods": ["embedContent"]
            }
        ]
    }))
    .into_response()
}

// 返回碎片化的 Gemini SSE 候选和单独的完成原因事件。
async fn gemini_stream(
    State(state): State<FakeProviderState>,
    headers: HeaderMap,
    Json(_request): Json<serde_json::Value>,
) -> Response {
    state.chat_request_counter.fetch_add(1, Ordering::SeqCst);
    if !authorized(&state, &headers) {
        return (StatusCode::UNAUTHORIZED, "Authentication failed.").into_response();
    }
    let output = stream! {
        for fragment in ["你好", "，", "Gemini", "！"] {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let data = json!({
                "candidates": [{
                    "content": {"parts": [{"text": fragment}]}
                }]
            });
            yield Ok::<Event, Infallible>(Event::default().data(data.to_string()));
        }
        let done = json!({"candidates": [{"finishReason": "STOP"}]});
        yield Ok::<Event, Infallible>(Event::default().data(done.to_string()));
    };
    Sse::new(output)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(5)))
        .into_response()
}

async fn models(State(state): State<FakeProviderState>, headers: HeaderMap) -> Response {
    state.model_request_counter.fetch_add(1, Ordering::SeqCst);
    if !state.model_delay.is_zero() {
        tokio::time::sleep(state.model_delay).await;
    }
    if !authorized(&state, &headers) {
        return (StatusCode::UNAUTHORIZED, "Authentication failed.").into_response();
    }
    let models = match state.flavor {
        FakeProviderFlavor::Standard | FakeProviderFlavor::Responses => json!([
            { "id": "fake-translator", "object": "model" },
            { "id": "fake-slow-translator", "object": "model" }
        ]),
        FakeProviderFlavor::Ollama | FakeProviderFlavor::OllamaNative => json!([
            { "id": "llama3.2:latest", "object": "model", "owned_by": "ollama" }
        ]),
        FakeProviderFlavor::Gemini | FakeProviderFlavor::Azure => json!([]),
    };
    Json(json!({ "object": "list", "data": models })).into_response()
}

async fn ollama_tags(State(state): State<FakeProviderState>, headers: HeaderMap) -> Response {
    state.model_request_counter.fetch_add(1, Ordering::SeqCst);
    if !authorized(&state, &headers) {
        return (StatusCode::UNAUTHORIZED, "Authentication failed.").into_response();
    }
    Json(json!({
        "models": [{ "name": "llama3.2:latest", "model": "llama3.2:latest" }]
    }))
    .into_response()
}

async fn ollama_chat(
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
    let malformed = source.contains("[malformed]");
    let disconnect = source.contains("[disconnect]");
    let output = stream! {
        for fragment in ["你好", "，", "Ollama", "！"] {
            tokio::time::sleep(Duration::from_millis(35)).await;
            if malformed {
                yield Ok::<Bytes, Infallible>(Bytes::from_static(b"{not-json\n"));
                return;
            }
            let line = json!({
                "message": { "role": "assistant", "content": fragment },
                "done": false
            });
            yield Ok::<Bytes, Infallible>(Bytes::from(format!("{line}\n")));
        }
        if !disconnect {
            yield Ok::<Bytes, Infallible>(Bytes::from_static(b"{\"done\":true}\n"));
        }
    };
    Response::builder()
        .header("content-type", "application/x-ndjson")
        .body(Body::from_stream(output))
        .expect("Ollama response")
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
        let fragments = match state.flavor {
            FakeProviderFlavor::Standard => ["你好", "，", "LinguaMesh", "！"],
            FakeProviderFlavor::Ollama | FakeProviderFlavor::OllamaNative => {
                ["你好", "，", "Ollama", "！"]
            }
            FakeProviderFlavor::Gemini => ["你好", "，", "Gemini", "！"],
            FakeProviderFlavor::Azure => ["你好", "，", "Azure", "！"],
            FakeProviderFlavor::Responses => ["你好", "，", "Responses", "！"],
        };
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

// 返回 OpenAI Responses API 的 typed SSE 事件序列。
async fn responses(
    State(state): State<FakeProviderState>,
    headers: HeaderMap,
    Json(_request): Json<serde_json::Value>,
) -> Response {
    state.chat_request_counter.fetch_add(1, Ordering::SeqCst);
    if !authorized(&state, &headers) {
        return (StatusCode::UNAUTHORIZED, "Authentication failed.").into_response();
    }
    let output = stream! {
        yield Ok::<Event, Infallible>(Event::default()
            .event("response.created")
            .data(json!({"type": "response.created"}).to_string()));
        for fragment in ["你好", "，", "Responses", "！"] {
            tokio::time::sleep(Duration::from_millis(20)).await;
            yield Ok::<Event, Infallible>(Event::default()
                .event("response.output_text.delta")
                .data(json!({
                    "type": "response.output_text.delta",
                    "delta": fragment
                }).to_string()));
        }
        yield Ok::<Event, Infallible>(Event::default()
            .event("response.completed")
            .data(json!({"type": "response.completed"}).to_string()));
    };
    Sse::new(output)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(5)))
        .into_response()
}

// 复用 OpenAI 兼容响应体，仅让路由和认证头体现 Azure 协议差异。
async fn azure_chat_completions(
    state: State<FakeProviderState>,
    headers: HeaderMap,
    request: Json<FakeChatRequest>,
) -> Response {
    chat_completions(state, headers, request).await
}

fn authorized(state: &FakeProviderState, headers: &HeaderMap) -> bool {
    let token_authorized = state.expected_token.as_ref().is_none_or(|expected| {
        if matches!(state.flavor, FakeProviderFlavor::Azure) {
            headers
                .get("api-key")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value == expected.expose_secret())
        } else {
            headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "))
                .is_some_and(|value| value == expected.expose_secret())
        }
    });
    token_authorized
        && state.expected_project.as_deref().is_none_or(|expected| {
            headers
                .get("OpenAI-Project")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value == expected)
        })
        && state
            .expected_custom_header
            .as_ref()
            .is_none_or(|(name, expected)| {
                headers
                    .get(name)
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| value == expected)
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

    #[tokio::test]
    async fn ollama_compatible_server_exposes_openai_model_shape() {
        let server = FakeProviderServer::start_ollama_compatible()
            .await
            .expect("server");
        let response = reqwest::Client::new()
            .get(format!("{}models", server.base_url()))
            .send()
            .await
            .expect("models request");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let body: serde_json::Value = response.json().await.expect("models response");
        assert_eq!(body["data"][0]["id"], "llama3.2:latest");
        assert_eq!(body["data"][0]["owned_by"], "ollama");
        server.shutdown().await;
    }

    #[tokio::test]
    async fn ollama_native_server_exposes_tags_and_ndjson_chat() {
        let server = FakeProviderServer::start_ollama_native()
            .await
            .expect("server");
        let client = reqwest::Client::new();
        let tags: serde_json::Value = client
            .get(format!("{}tags", server.ollama_base_url()))
            .send()
            .await
            .expect("tags request")
            .json()
            .await
            .expect("tags response");
        assert_eq!(tags["models"][0]["name"], "llama3.2:latest");
        let response = client
            .post(format!("{}chat", server.ollama_base_url()))
            .json(&serde_json::json!({
                "model": "llama3.2:latest",
                "stream": true,
                "messages": [{"role": "user", "content": "Hello"}]
            }))
            .send()
            .await
            .expect("chat request");
        let body = response.text().await.expect("chat response");
        assert!(body.contains("你好"));
        assert!(body.contains("\"done\":true"));
        server.shutdown().await;
    }

    #[tokio::test]
    async fn gemini_server_exposes_models_and_sse_stream() {
        let server = FakeProviderServer::start_gemini().await.expect("server");
        let client = reqwest::Client::new();
        let models: serde_json::Value = client
            .get(format!("{}models", server.gemini_base_url()))
            .send()
            .await
            .expect("models request")
            .json()
            .await
            .expect("models response");
        assert_eq!(models["models"][0]["name"], "models/gemini-2.0-flash");
        assert_eq!(
            models["models"][1]["supportedGenerationMethods"][0],
            "embedContent"
        );
        let body = client
            .post(format!(
                "{}models/gemini-2.0-flash:streamGenerateContent?alt=sse",
                server.gemini_base_url()
            ))
            .json(&serde_json::json!({"contents": [{"parts": [{"text": "Hello"}]}]}))
            .send()
            .await
            .expect("stream request")
            .text()
            .await
            .expect("stream response");
        assert!(body.contains("Gemini"));
        assert!(body.contains("finishReason"));
        server.shutdown().await;
    }

    #[tokio::test]
    async fn azure_server_requires_api_key_and_uses_deployment_path() {
        let server = FakeProviderServer::start_azure().await.expect("server");
        let client = reqwest::Client::new();
        let endpoint = format!(
            "{}openai/deployments/fake-deployment/chat/completions?api-version=2024-10-21",
            server.azure_base_url()
        );
        let unauthorized = client
            .post(&endpoint)
            .json(&serde_json::json!({
                "model": "fake-deployment",
                "messages": [{"role": "user", "content": "Hello"}]
            }))
            .send()
            .await
            .expect("unauthorized request");
        assert_eq!(unauthorized.status(), reqwest::StatusCode::UNAUTHORIZED);
        let response = client
            .post(endpoint)
            .header("api-key", "azure-test-key")
            .json(&serde_json::json!({
                "model": "fake-deployment",
                "messages": [{"role": "user", "content": "Hello"}]
            }))
            .send()
            .await
            .expect("authorized request");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert!(
            response
                .text()
                .await
                .expect("stream response")
                .contains("Azure")
        );
        server.shutdown().await;
    }
}
