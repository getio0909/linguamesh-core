#![doc = "`LinguaMesh` 的稳定 C ABI 实现。"]
#![allow(clippy::missing_safety_doc)]

use linguamesh_application::{HostSecretBroker, SecretRequestLease, host_secret_channel};
use linguamesh_domain::{
    CorrelationId, ErrorKind, FileLease, FileLeaseError, OperationId, SecretRef, SecretValue,
    TranslationEvent, TranslationRequest,
};
use linguamesh_engine::{
    CancellationHandle, TranslationEngine, TranslationOperation, core_compatibility,
};
use linguamesh_protocol::{
    ABI_VERSION_MAJOR, CompatibilitySnapshot, Envelope, FailureEvent, HostSecretResponse,
    PROTOCOL_VERSION, SecretRequiredEvent, TextDeltaEvent, TranslateTextCommand, message_type,
};
use linguamesh_provider_openai::{OpenAiCompatibleProvider, OpenAiConfig};
use prost::Message;
use std::collections::{HashMap, hash_map::Entry};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};
use tokio_util::sync::CancellationToken;

const EVENT_QUEUE_CAPACITY: usize = 64;
const MAX_OUTSTANDING_BUFFERS: usize = 64;
const MAX_FILE_LEASES: usize = 64;
const MAX_FILE_LEASE_LOCATION_BYTES: usize = 4096;
const MAX_PROTOCOL_MESSAGE_BYTES: usize = 1024 * 1024;
const MAX_HOST_SECRET_BYTES: usize = 64 * 1024;

/// 表示 ABI 调用结果。
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LmResultCode {
    /// 调用成功。
    Ok = 0,
    /// 指针或参数无效。
    InvalidArgument = 1,
    /// 引擎已经关闭。
    Shutdown = 2,
    /// Rust 内部发生恐慌并被边界捕获。
    Panic = 3,
    /// 消息使用不兼容的协议版本。
    ProtocolIncompatible = 4,
    /// 消息无法解码或缺少必要字段。
    MalformedMessage = 5,
    /// 引擎已有活动操作。
    Busy = 6,
    /// 消息类型当前不受支持。
    UnsupportedMessage = 7,
    /// 运行时资源无法创建。
    Internal = 8,
    /// 引擎已达到未释放缓冲区上限。
    ResourceExhausted = 9,
}

/// 表示由 Rust 分配且必须显式释放的字节缓冲区。
#[repr(C)]
pub struct LmBuffer {
    /// 字节起始地址。
    pub data: *mut u8,
    /// 有效字节数。
    pub len: usize,
    /// 分配容量。
    pub capacity: usize,
    /// 防止地址复用误释放的分配令牌。
    pub allocation_id: u64,
}

struct ActiveOperation {
    operation_id: String,
    cancellation: CancellationHandle,
}

struct BufferAllocation {
    bytes: Vec<u8>,
    _slot: OwnedSemaphorePermit,
}

struct PendingHostRequest {
    lease: SecretRequestLease,
    operation_id: String,
    correlation_id: String,
}

/// 表示不向调用方暴露内部布局的引擎句柄。
pub struct LmEngine {
    shutdown: AtomicBool,
    runtime: Runtime,
    event_sender: mpsc::Sender<Vec<u8>>,
    events: tokio::sync::Mutex<mpsc::Receiver<Vec<u8>>>,
    active: Arc<Mutex<Option<ActiveOperation>>>,
    pending_host_requests: Arc<Mutex<HashMap<String, PendingHostRequest>>>,
    file_leases: Mutex<HashMap<u64, FileLease>>,
    buffer_allocations: Mutex<HashMap<u64, BufferAllocation>>,
    buffer_slots: Arc<Semaphore>,
    next_buffer_allocation_id: AtomicU64,
    next_file_lease_id: AtomicU64,
}

/// 创建新的不透明引擎句柄。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lm_engine_create(output: *mut *mut LmEngine) -> LmResultCode {
    ffi_guard(|| {
        if output.is_null() {
            return LmResultCode::InvalidArgument;
        }
        // SAFETY：调用方提供了经过非空验证且可写的输出指针。
        unsafe { output.write(ptr::null_mut()) };
        let Ok(runtime) = Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("linguamesh-ffi")
            .enable_all()
            .build()
        else {
            return LmResultCode::Internal;
        };
        let (event_sender, events) = mpsc::channel(EVENT_QUEUE_CAPACITY);
        let engine = Box::new(LmEngine {
            shutdown: AtomicBool::new(false),
            runtime,
            event_sender,
            events: tokio::sync::Mutex::new(events),
            active: Arc::new(Mutex::new(None)),
            pending_host_requests: Arc::new(Mutex::new(HashMap::new())),
            file_leases: Mutex::new(HashMap::new()),
            buffer_allocations: Mutex::new(HashMap::new()),
            buffer_slots: Arc::new(Semaphore::new(MAX_OUTSTANDING_BUFFERS)),
            next_buffer_allocation_id: AtomicU64::new(1),
            next_file_lease_id: AtomicU64::new(1),
        });
        // SAFETY：调用方提供了经过非空验证且可写的输出指针。
        unsafe { output.write(Box::into_raw(engine)) };
        LmResultCode::Ok
    })
}

/// 返回当前 C ABI 主版本。
#[unsafe(no_mangle)]
pub const extern "C" fn lm_engine_get_abi_version() -> u32 {
    ABI_VERSION_MAJOR
}

/// 返回当前命令和事件协议版本。
#[unsafe(no_mangle)]
pub const extern "C" fn lm_engine_get_protocol_version() -> u32 {
    PROTOCOL_VERSION
}

/// 保留旧的协议版本查询符号。
#[unsafe(no_mangle)]
pub const extern "C" fn lm_engine_get_version() -> u32 {
    lm_engine_get_protocol_version()
}

/// 返回版本化共享核心兼容性快照，并沿用引擎拥有的缓冲区释放协议。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lm_engine_get_compatibility(
    engine: *mut LmEngine,
    output: *mut LmBuffer,
) -> LmResultCode {
    ffi_guard(|| {
        if output.is_null() {
            return LmResultCode::InvalidArgument;
        }
        // SAFETY：输出描述符已经验证为非空并由调用方授予可写访问。
        let output = unsafe { &mut *output };
        if !output.data.is_null()
            || output.len != 0
            || output.capacity != 0
            || output.allocation_id != 0
        {
            return LmResultCode::InvalidArgument;
        }
        // SAFETY：调用方保证非空句柄由本库创建且尚未销毁。
        let Some(engine) = (unsafe { engine_ref(engine) }) else {
            return LmResultCode::InvalidArgument;
        };
        if engine.shutdown.load(Ordering::Acquire) {
            return LmResultCode::Shutdown;
        }
        let Ok(buffer_slot) = engine.reserve_buffer_slot() else {
            return LmResultCode::ResourceExhausted;
        };
        let Ok(compatibility) = core_compatibility() else {
            return LmResultCode::Internal;
        };
        let envelope = Envelope {
            protocol_version: PROTOCOL_VERSION,
            operation_id: OperationId::new().as_str().to_owned(),
            correlation_id: CorrelationId::new().as_str().to_owned(),
            sequence: 0,
            message_type: message_type::COMPATIBILITY.into(),
            payload: CompatibilitySnapshot {
                core_version: compatibility.core_version,
                abi_major: compatibility.abi_major,
                protocol_version: compatibility.protocol_version,
                provider_catalog_version: compatibility.provider_catalog_version,
                enabled_features: compatibility.enabled_features,
            }
            .encode_to_vec(),
        }
        .encode_to_vec();
        engine.write_output_buffer(output, envelope, buffer_slot)
    })
}

/// 在引擎内注册桌面路径文件 lease，并返回不透明数字令牌。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lm_engine_file_lease_create_desktop_path(
    engine: *mut LmEngine,
    data: *const u8,
    len: usize,
    output: *mut u64,
) -> LmResultCode {
    create_path_file_lease(engine, data, len, output, FileLease::desktop_path)
}

/// 在引擎内注册临时文件路径 lease，并返回不透明数字令牌。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lm_engine_file_lease_create_temporary_path(
    engine: *mut LmEngine,
    data: *const u8,
    len: usize,
    output: *mut u64,
) -> LmResultCode {
    create_path_file_lease(engine, data, len, output, FileLease::temporary_path)
}

/// 在引擎内注册输出目标路径 lease，并返回不透明数字令牌。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lm_engine_file_lease_create_output_path(
    engine: *mut LmEngine,
    data: *const u8,
    len: usize,
    output: *mut u64,
) -> LmResultCode {
    create_path_file_lease(engine, data, len, output, FileLease::output_path)
}

/// 在引擎内注册 POSIX 文件描述符 lease，并返回不透明数字令牌。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lm_engine_file_lease_create_posix_descriptor(
    engine: *mut LmEngine,
    descriptor: i64,
    output: *mut u64,
) -> LmResultCode {
    ffi_guard(|| {
        let Some(engine) = (unsafe { engine_ref(engine) }) else {
            return LmResultCode::InvalidArgument;
        };
        engine.create_file_lease(output, FileLease::posix_descriptor(descriptor))
    })
}

/// 在引擎内注册 Android `ParcelFileDescriptor` 导出的 lease，并返回令牌。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lm_engine_file_lease_create_android_parcel_descriptor(
    engine: *mut LmEngine,
    descriptor: i64,
    output: *mut u64,
) -> LmResultCode {
    ffi_guard(|| {
        let Some(engine) = (unsafe { engine_ref(engine) }) else {
            return LmResultCode::InvalidArgument;
        };
        engine.create_file_lease(output, FileLease::android_parcel_descriptor(descriptor))
    })
}

/// 在引擎内注册 Windows 复制句柄 lease，并返回不透明数字令牌。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lm_engine_file_lease_create_windows_handle(
    engine: *mut LmEngine,
    handle: u64,
    output: *mut u64,
) -> LmResultCode {
    ffi_guard(|| {
        let Some(engine) = (unsafe { engine_ref(engine) }) else {
            return LmResultCode::InvalidArgument;
        };
        engine.create_file_lease(output, FileLease::windows_handle(handle))
    })
}

/// 查询引擎内 lease 是否仍然有效。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lm_engine_file_lease_is_active(
    engine: *mut LmEngine,
    lease_id: u64,
    output: *mut u8,
) -> LmResultCode {
    ffi_guard(|| {
        if lease_id == 0 || output.is_null() {
            return LmResultCode::InvalidArgument;
        }
        let Some(engine) = (unsafe { engine_ref(engine) }) else {
            return LmResultCode::InvalidArgument;
        };
        let leases = lock_unpoisoned(&engine.file_leases);
        let Some(lease) = leases.get(&lease_id) else {
            return LmResultCode::InvalidArgument;
        };
        // SAFETY：输出指针经过非空验证，调用方负责提供一个可写字节。
        unsafe { output.write(u8::from(lease.is_active())) };
        LmResultCode::Ok
    })
}

/// 使引擎内 lease 到期；到期后任何借用都必须失败。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lm_engine_file_lease_expire(
    engine: *mut LmEngine,
    lease_id: u64,
) -> LmResultCode {
    update_file_lease_state(engine, lease_id, FileLease::expire)
}

/// 撤销引擎内 lease；撤销状态不可恢复。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lm_engine_file_lease_revoke(
    engine: *mut LmEngine,
    lease_id: u64,
) -> LmResultCode {
    update_file_lease_state(engine, lease_id, FileLease::revoke)
}

/// 删除引擎内 lease；删除前先执行不可逆撤销。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lm_engine_file_lease_destroy(
    engine: *mut LmEngine,
    lease_id: u64,
) -> LmResultCode {
    ffi_guard(|| {
        if lease_id == 0 {
            return LmResultCode::InvalidArgument;
        }
        let Some(engine) = (unsafe { engine_ref(engine) }) else {
            return LmResultCode::InvalidArgument;
        };
        let mut leases = lock_unpoisoned(&engine.file_leases);
        let Some(lease) = leases.remove(&lease_id) else {
            return LmResultCode::InvalidArgument;
        };
        lease.revoke();
        LmResultCode::Ok
    })
}

/// 验证并提交版本化命令消息。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lm_engine_submit(
    engine: *mut LmEngine,
    command_data: *const u8,
    command_len: usize,
) -> LmResultCode {
    ffi_guard(|| {
        // SAFETY：调用方保证句柄和消息缓冲区在本次同步调用期间有效。
        let envelope = match unsafe { decode_protocol_input(engine, command_data, command_len) } {
            Ok(envelope) => envelope,
            Err(code) => return code,
        };
        // SAFETY：解码成功证明句柄非空且在本次调用期间有效。
        let engine = unsafe { &*engine };
        engine.submit(&envelope)
    })
}

/// 验证并接收版本化主机响应消息。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lm_engine_send_host_response(
    engine: *mut LmEngine,
    response_data: *const u8,
    response_len: usize,
) -> LmResultCode {
    ffi_guard(|| {
        // SAFETY：调用方保证句柄和消息缓冲区在本次同步调用期间有效。
        let envelope = match unsafe { decode_protocol_input(engine, response_data, response_len) } {
            Ok(envelope) => envelope,
            Err(code) => return code,
        };
        // SAFETY：解码成功证明句柄非空且在本次调用期间有效。
        let engine = unsafe { &*engine };
        engine.send_host_response(&envelope)
    })
}

/// 标记当前操作应当取消。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lm_engine_cancel(engine: *mut LmEngine) -> LmResultCode {
    ffi_guard(|| {
        // SAFETY：调用方保证非空句柄由本库创建且尚未销毁。
        let Some(engine) = (unsafe { engine_ref(engine) }) else {
            return LmResultCode::InvalidArgument;
        };
        if engine.shutdown.load(Ordering::Acquire) {
            return LmResultCode::Shutdown;
        }
        if let Some(active) = lock_unpoisoned(&engine.active).as_ref() {
            active.cancellation.cancel();
        }
        LmResultCode::Ok
    })
}

/// 以有界超时轮询下一条事件。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lm_engine_poll_event(
    engine: *mut LmEngine,
    timeout_ms: u32,
    output: *mut LmBuffer,
) -> LmResultCode {
    ffi_guard(|| {
        if output.is_null() {
            return LmResultCode::InvalidArgument;
        }
        // SAFETY：输出描述符已经验证为非空并由调用方授予可写访问。
        let output = unsafe { &mut *output };
        if !output.data.is_null()
            || output.len != 0
            || output.capacity != 0
            || output.allocation_id != 0
        {
            return LmResultCode::InvalidArgument;
        }
        // SAFETY：调用方保证非空句柄由本库创建且尚未销毁。
        let Some(engine) = (unsafe { engine_ref(engine) }) else {
            return LmResultCode::InvalidArgument;
        };
        if engine.shutdown.load(Ordering::Acquire) {
            return LmResultCode::Shutdown;
        }
        let Ok(buffer_slot) = engine.reserve_buffer_slot() else {
            return LmResultCode::ResourceExhausted;
        };
        let event = if timeout_ms == 0 {
            engine
                .events
                .try_lock()
                .ok()
                .and_then(|mut events| events.try_recv().ok())
        } else {
            engine.runtime.block_on(async {
                tokio::time::timeout(Duration::from_millis(u64::from(timeout_ms)), async {
                    let mut events = engine.events.lock().await;
                    events.recv().await
                })
                .await
                .ok()
                .flatten()
            })
        };
        engine.write_output_buffer(output, event.unwrap_or_default(), buffer_slot)
    })
}

/// 请求引擎停止接受新工作。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lm_engine_shutdown(engine: *mut LmEngine) -> LmResultCode {
    ffi_guard(|| {
        // SAFETY：调用方保证非空句柄由本库创建且尚未销毁。
        let Some(engine) = (unsafe { engine_ref(engine) }) else {
            return LmResultCode::InvalidArgument;
        };
        engine.shutdown.store(true, Ordering::Release);
        if let Some(active) = lock_unpoisoned(&engine.active).as_ref() {
            active.cancellation.cancel();
        }
        LmResultCode::Ok
    })
}

/// 销毁不透明引擎句柄。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lm_engine_destroy(engine: *mut LmEngine) -> LmResultCode {
    ffi_guard(|| {
        if engine.is_null() {
            return LmResultCode::InvalidArgument;
        }
        // SAFETY：调用方必须仅传入由 lm_engine_create 返回且尚未销毁的句柄。
        let engine = unsafe { Box::from_raw(engine) };
        engine.shutdown.store(true, Ordering::Release);
        if let Some(active) = lock_unpoisoned(&engine.active).as_ref() {
            active.cancellation.cancel();
        }
        drop(engine);
        LmResultCode::Ok
    })
}

/// 通过所属引擎释放 Rust 分配的缓冲区并清空描述符。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lm_engine_buffer_free(
    engine: *mut LmEngine,
    buffer: *mut LmBuffer,
) -> LmResultCode {
    ffi_guard(|| {
        if buffer.is_null() {
            return LmResultCode::InvalidArgument;
        }
        // SAFETY：调用方保证非空句柄由本库创建且尚未销毁。
        let Some(engine) = (unsafe { engine_ref(engine) }) else {
            return LmResultCode::InvalidArgument;
        };
        // SAFETY：缓冲区描述符已经验证为非空并由调用方授予可写访问。
        let buffer = unsafe { &mut *buffer };
        if buffer.data.is_null() {
            if buffer.len == 0 && buffer.capacity == 0 && buffer.allocation_id == 0 {
                return LmResultCode::Ok;
            }
            return LmResultCode::InvalidArgument;
        }
        if buffer.len > buffer.capacity || buffer.allocation_id == 0 {
            return LmResultCode::InvalidArgument;
        }
        let mut allocations = lock_unpoisoned(&engine.buffer_allocations);
        let Some(allocation) = allocations.get(&buffer.allocation_id) else {
            return LmResultCode::InvalidArgument;
        };
        if !ptr::eq(allocation.bytes.as_ptr(), buffer.data.cast_const())
            || allocation.bytes.len() != buffer.len
            || allocation.bytes.capacity() != buffer.capacity
        {
            return LmResultCode::InvalidArgument;
        }
        let allocation = allocations
            .remove(&buffer.allocation_id)
            .expect("registered allocation");
        drop(allocations);
        drop(allocation);
        buffer.data = ptr::null_mut();
        buffer.len = 0;
        buffer.capacity = 0;
        buffer.allocation_id = 0;
        LmResultCode::Ok
    })
}

impl LmEngine {
    fn create_file_lease(
        &self,
        output: *mut u64,
        lease: Result<FileLease, FileLeaseError>,
    ) -> LmResultCode {
        if output.is_null() {
            return LmResultCode::InvalidArgument;
        }
        // SAFETY：输出指针经过非空验证，调用方负责提供一个可读写的令牌槽位。
        if unsafe { output.read() } != 0 {
            return LmResultCode::InvalidArgument;
        }
        if self.shutdown.load(Ordering::Acquire) {
            return LmResultCode::Shutdown;
        }
        let Ok(lease) = lease else {
            return LmResultCode::InvalidArgument;
        };
        let mut leases = lock_unpoisoned(&self.file_leases);
        if leases.len() >= MAX_FILE_LEASES {
            return LmResultCode::ResourceExhausted;
        }
        let lease_id = loop {
            let candidate = self.next_file_lease_id.fetch_add(1, Ordering::Relaxed);
            if candidate == 0 {
                continue;
            }
            if let Entry::Vacant(entry) = leases.entry(candidate) {
                entry.insert(lease);
                break candidate;
            }
        };
        drop(leases);
        // SAFETY：输出指针仍由调用方在本次同步调用中独占。
        unsafe { output.write(lease_id) };
        LmResultCode::Ok
    }

    fn submit(&self, envelope: &Envelope) -> LmResultCode {
        if self.shutdown.load(Ordering::Acquire) {
            return LmResultCode::Shutdown;
        }
        if envelope.message_type != message_type::TRANSLATE_TEXT {
            return LmResultCode::UnsupportedMessage;
        }
        if envelope.sequence != 0 {
            return LmResultCode::MalformedMessage;
        }
        let Ok(command) = TranslateTextCommand::decode(envelope.payload.as_slice()) else {
            return LmResultCode::MalformedMessage;
        };
        if command.endpoint.is_empty()
            || command.model_id.is_empty()
            || command.source_text.is_empty()
            || command.target_locale.is_empty()
        {
            return LmResultCode::MalformedMessage;
        }
        let secret_ref = if command.secret_ref.is_empty() {
            None
        } else {
            match SecretRef::parse(command.secret_ref.clone()) {
                Ok(secret_ref) => Some(secret_ref),
                Err(_) => return LmResultCode::MalformedMessage,
            }
        };
        let mut active = lock_unpoisoned(&self.active);
        if self.shutdown.load(Ordering::Acquire) {
            return LmResultCode::Shutdown;
        }
        if active.is_some() {
            return LmResultCode::Busy;
        }
        if let Some(secret_ref) = secret_ref {
            drop(active);
            return self.submit_with_host_secret(envelope, command, secret_ref);
        }
        let Ok(provider) =
            OpenAiCompatibleProvider::new(OpenAiConfig::without_credential(command.endpoint))
        else {
            return LmResultCode::InvalidArgument;
        };
        let mut request =
            TranslationRequest::new(command.source_text, command.target_locale, command.model_id);
        request.operation_id = OperationId::from_value(envelope.operation_id.clone());
        request.correlation_id = CorrelationId::from_value(envelope.correlation_id.clone());
        let translation_engine = TranslationEngine::new(Arc::new(provider));
        let operation = {
            let _runtime_context = self.runtime.enter();
            translation_engine.translate(request)
        };
        *active = Some(ActiveOperation {
            operation_id: envelope.operation_id.clone(),
            cancellation: operation.cancellation_handle(),
        });
        drop(active);
        let sender = self.event_sender.clone();
        let active = Arc::clone(&self.active);
        self.runtime.spawn(forward_events(
            envelope.operation_id.clone(),
            envelope.correlation_id.clone(),
            operation,
            sender,
            active,
        ));
        LmResultCode::Ok
    }

    fn reserve_buffer_slot(&self) -> Result<OwnedSemaphorePermit, LmResultCode> {
        Arc::clone(&self.buffer_slots)
            .try_acquire_owned()
            .map_err(|_| LmResultCode::ResourceExhausted)
    }

    fn write_output_buffer(
        &self,
        output: &mut LmBuffer,
        mut bytes: Vec<u8>,
        buffer_slot: OwnedSemaphorePermit,
    ) -> LmResultCode {
        if bytes.is_empty() {
            *output = LmBuffer {
                data: ptr::null_mut(),
                len: 0,
                capacity: 0,
                allocation_id: 0,
            };
            return LmResultCode::Ok;
        }
        let data = bytes.as_mut_ptr();
        let len = bytes.len();
        let capacity = bytes.capacity();
        let mut allocations = lock_unpoisoned(&self.buffer_allocations);
        let allocation_id = loop {
            let candidate = self
                .next_buffer_allocation_id
                .fetch_add(1, Ordering::Relaxed);
            if candidate == 0 {
                continue;
            }
            match allocations.entry(candidate) {
                Entry::Vacant(entry) => {
                    entry.insert(BufferAllocation {
                        bytes,
                        _slot: buffer_slot,
                    });
                    break candidate;
                }
                Entry::Occupied(_) => {}
            }
        };
        drop(allocations);
        *output = LmBuffer {
            data,
            len,
            capacity,
            allocation_id,
        };
        LmResultCode::Ok
    }

    fn submit_with_host_secret(
        &self,
        envelope: &Envelope,
        command: TranslateTextCommand,
        secret_ref: SecretRef,
    ) -> LmResultCode {
        if OpenAiCompatibleProvider::validate_endpoint(&command.endpoint).is_err() {
            return LmResultCode::InvalidArgument;
        }
        let Ok((broker, requests)) = host_secret_channel(1) else {
            return LmResultCode::Internal;
        };
        let cancellation = CancellationToken::new();
        let operation_id = envelope.operation_id.clone();
        let correlation_id = envelope.correlation_id.clone();
        let sender = self.event_sender.clone();
        let pending = Arc::clone(&self.pending_host_requests);
        let request_operation_id = operation_id.clone();
        let request_correlation_id = correlation_id.clone();
        let request_pending = Arc::clone(&pending);
        let request_task = async move {
            let mut requests = requests;
            while let Some(lease) = requests.recv().await {
                let required = lease.required().clone();
                lock_unpoisoned(&request_pending).insert(
                    required.request_id.as_str().to_owned(),
                    PendingHostRequest {
                        lease,
                        operation_id: request_operation_id.clone(),
                        correlation_id: request_correlation_id.clone(),
                    },
                );
                let event = Envelope {
                    protocol_version: PROTOCOL_VERSION,
                    operation_id: request_operation_id.clone(),
                    correlation_id: request_correlation_id.clone(),
                    sequence: 0,
                    message_type: message_type::SECRET_REQUIRED.into(),
                    payload: SecretRequiredEvent {
                        request_id: required.request_id.as_str().into(),
                        secret_ref: required.secret_ref.as_str().into(),
                    }
                    .encode_to_vec(),
                }
                .encode_to_vec();
                if sender.send(event).await.is_err() {
                    break;
                }
            }
        };
        self.runtime.spawn(request_task);
        {
            let mut active = lock_unpoisoned(&self.active);
            if active.is_some() {
                return LmResultCode::Busy;
            }
            *active = Some(ActiveOperation {
                operation_id: operation_id.clone(),
                cancellation: CancellationHandle::from_token(cancellation.clone()),
            });
        }
        let sender = self.event_sender.clone();
        let active = Arc::clone(&self.active);
        self.runtime
            .spawn(run_host_secret_translation(HostSecretTranslation {
                operation_id,
                correlation_id,
                command,
                secret_ref,
                broker,
                cancellation,
                sender,
                active,
                pending,
            }));
        LmResultCode::Ok
    }

    fn send_host_response(&self, envelope: &Envelope) -> LmResultCode {
        if envelope.message_type != message_type::HOST_SECRET_RESPONSE {
            return LmResultCode::UnsupportedMessage;
        }
        if envelope.sequence != 0 {
            return LmResultCode::MalformedMessage;
        }
        let active = lock_unpoisoned(&self.active);
        if active.as_ref().is_none_or(|value| {
            value.operation_id != envelope.operation_id || envelope.correlation_id.is_empty()
        }) {
            return LmResultCode::InvalidArgument;
        }
        drop(active);
        let Ok(response) = HostSecretResponse::decode(envelope.payload.as_slice()) else {
            return LmResultCode::MalformedMessage;
        };
        if response.request_id.is_empty() || response.request_id.len() > 128 {
            return LmResultCode::MalformedMessage;
        }
        let valid_resolution = match response.resolution.as_str() {
            "provided" => {
                !response.secret.is_empty() && response.secret.len() <= MAX_HOST_SECRET_BYTES
            }
            "unavailable" | "secure_storage_unavailable" => response.secret.is_empty(),
            _ => false,
        };
        if !valid_resolution {
            return LmResultCode::MalformedMessage;
        }
        let mut pending_requests = lock_unpoisoned(&self.pending_host_requests);
        let Some(pending) = pending_requests.get(&response.request_id) else {
            return LmResultCode::InvalidArgument;
        };
        if pending.operation_id != envelope.operation_id
            || pending.correlation_id != envelope.correlation_id
        {
            return LmResultCode::InvalidArgument;
        }
        let pending = pending_requests
            .remove(&response.request_id)
            .expect("validated pending host request");
        drop(pending_requests);
        let lease = pending.lease;
        match response.resolution.as_str() {
            "provided" if !response.secret.is_empty() => lease
                .provide_secret(SecretValue::new(response.secret))
                .map_or(LmResultCode::InvalidArgument, |()| LmResultCode::Ok),
            "unavailable" if response.secret.is_empty() => lease
                .reject_unavailable()
                .map_or(LmResultCode::InvalidArgument, |()| LmResultCode::Ok),
            "secure_storage_unavailable" if response.secret.is_empty() => lease
                .reject_secure_storage_unavailable()
                .map_or(LmResultCode::InvalidArgument, |()| LmResultCode::Ok),
            _ => LmResultCode::MalformedMessage,
        }
    }
}

fn create_path_file_lease(
    engine: *mut LmEngine,
    data: *const u8,
    len: usize,
    output: *mut u64,
    constructor: fn(String) -> Result<FileLease, FileLeaseError>,
) -> LmResultCode {
    ffi_guard(|| {
        if data.is_null() || len == 0 || len > MAX_FILE_LEASE_LOCATION_BYTES {
            return LmResultCode::InvalidArgument;
        }
        // SAFETY：调用方保证数据指针在本次同步调用期间可读，长度已受上限约束。
        let bytes = unsafe { std::slice::from_raw_parts(data, len) };
        let Ok(value) = String::from_utf8(bytes.to_vec()) else {
            return LmResultCode::InvalidArgument;
        };
        let Some(engine) = (unsafe { engine_ref(engine) }) else {
            return LmResultCode::InvalidArgument;
        };
        engine.create_file_lease(output, constructor(value))
    })
}

fn update_file_lease_state(
    engine: *mut LmEngine,
    lease_id: u64,
    update: impl FnOnce(&FileLease),
) -> LmResultCode {
    ffi_guard(|| {
        if lease_id == 0 {
            return LmResultCode::InvalidArgument;
        }
        let Some(engine) = (unsafe { engine_ref(engine) }) else {
            return LmResultCode::InvalidArgument;
        };
        let leases = lock_unpoisoned(&engine.file_leases);
        let Some(lease) = leases.get(&lease_id) else {
            return LmResultCode::InvalidArgument;
        };
        update(lease);
        LmResultCode::Ok
    })
}

async fn forward_events(
    operation_id: String,
    correlation_id: String,
    mut operation: TranslationOperation,
    sender: mpsc::Sender<Vec<u8>>,
    active: Arc<Mutex<Option<ActiveOperation>>>,
) {
    while let Some(event) = operation.next_event().await {
        let encoded = encode_event(&operation_id, &correlation_id, event);
        if sender.send(encoded).await.is_err() {
            operation.cancel();
            break;
        }
    }
    let mut current = lock_unpoisoned(&active);
    if current
        .as_ref()
        .is_some_and(|value| value.operation_id == operation_id)
    {
        *current = None;
    }
}

struct HostSecretTranslation {
    operation_id: String,
    correlation_id: String,
    command: TranslateTextCommand,
    secret_ref: SecretRef,
    broker: HostSecretBroker,
    cancellation: CancellationToken,
    sender: mpsc::Sender<Vec<u8>>,
    active: Arc<Mutex<Option<ActiveOperation>>>,
    pending: Arc<Mutex<HashMap<String, PendingHostRequest>>>,
}

async fn run_host_secret_translation(context: HostSecretTranslation) {
    let HostSecretTranslation {
        operation_id,
        correlation_id,
        command,
        secret_ref,
        broker,
        cancellation,
        sender,
        active,
        pending,
    } = context;
    let credential = broker.resolve(&secret_ref, &cancellation).await;
    let secret = match credential {
        Ok(secret) => secret,
        Err(error) => {
            let event = if error.kind == ErrorKind::Cancelled {
                TranslationEvent::Cancelled { sequence: 1 }
            } else {
                TranslationEvent::Failed { sequence: 1, error }
            };
            let _ = sender
                .send(encode_event(&operation_id, &correlation_id, event))
                .await;
            lock_unpoisoned(&pending).clear();
            clear_active(&active, &operation_id);
            return;
        }
    };
    let Ok(provider) =
        OpenAiCompatibleProvider::new(OpenAiConfig::with_credential(command.endpoint, secret))
    else {
        let error = linguamesh_domain::TranslationError::new(
            ErrorKind::InvalidEndpoint,
            "The provider endpoint is invalid or unsafe.",
        );
        let _ = sender
            .send(encode_event(
                &operation_id,
                &correlation_id,
                TranslationEvent::Failed { sequence: 1, error },
            ))
            .await;
        lock_unpoisoned(&pending).clear();
        clear_active(&active, &operation_id);
        return;
    };
    let mut request =
        TranslationRequest::new(command.source_text, command.target_locale, command.model_id);
    request.operation_id = OperationId::from_value(operation_id.clone());
    request.correlation_id = CorrelationId::from_value(correlation_id.clone());
    let translation_engine = TranslationEngine::new(Arc::new(provider));
    let operation = translation_engine.translate_with_sequence_offset(request, 1);
    let operation_cancellation = operation.cancellation_handle();
    {
        let mut current = lock_unpoisoned(&active);
        if current
            .as_ref()
            .is_some_and(|value| value.operation_id == operation_id)
        {
            *current = Some(ActiveOperation {
                operation_id: operation_id.clone(),
                cancellation: operation_cancellation,
            });
        }
    }
    forward_events(
        operation_id.clone(),
        correlation_id,
        operation,
        sender,
        Arc::clone(&active),
    )
    .await;
    lock_unpoisoned(&pending).clear();
}

fn clear_active(active: &Arc<Mutex<Option<ActiveOperation>>>, operation_id: &str) {
    let mut current = lock_unpoisoned(active);
    if current
        .as_ref()
        .is_some_and(|value| value.operation_id == operation_id)
    {
        *current = None;
    }
}

fn encode_event(operation_id: &str, correlation_id: &str, event: TranslationEvent) -> Vec<u8> {
    let sequence = event.sequence();
    let (kind, payload) = match event {
        TranslationEvent::Started { .. } => (message_type::STARTED, Vec::new()),
        TranslationEvent::TextDelta { text, .. } => (
            message_type::TEXT_DELTA,
            TextDeltaEvent { text }.encode_to_vec(),
        ),
        TranslationEvent::Completed { .. } => (message_type::COMPLETED, Vec::new()),
        TranslationEvent::Cancelled { .. } => (message_type::CANCELLED, Vec::new()),
        TranslationEvent::Failed { error, .. } => (
            message_type::FAILED,
            FailureEvent {
                error_kind: error_kind_name(error.kind).into(),
                message: error.message,
            }
            .encode_to_vec(),
        ),
    };
    Envelope {
        protocol_version: PROTOCOL_VERSION,
        operation_id: operation_id.into(),
        correlation_id: correlation_id.into(),
        sequence,
        message_type: kind.into(),
        payload,
    }
    .encode_to_vec()
}

const fn error_kind_name(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Cancelled => "cancelled",
        ErrorKind::InvalidEndpoint => "invalid_endpoint",
        ErrorKind::Network => "network",
        ErrorKind::Timeout => "timeout",
        ErrorKind::Authentication => "authentication",
        ErrorKind::ModelUnavailable => "model_unavailable",
        ErrorKind::MalformedResponse => "malformed_response",
        ErrorKind::Persistence => "persistence",
        ErrorKind::ProtocolIncompatible => "protocol_incompatible",
        ErrorKind::InvalidConfiguration => "invalid_configuration",
        ErrorKind::UnsupportedCapability => "unsupported_capability",
        ErrorKind::SecretUnavailable => "secret_unavailable",
        ErrorKind::SecureStorageUnavailable => "secure_storage_unavailable",
        ErrorKind::Internal => "internal",
    }
}

unsafe fn engine_ref<'a>(engine: *mut LmEngine) -> Option<&'a LmEngine> {
    if engine.is_null() {
        None
    } else {
        // SAFETY：非空句柄的生命周期由调用方保证覆盖本次同步调用。
        Some(unsafe { &*engine })
    }
}

unsafe fn decode_protocol_input(
    engine: *mut LmEngine,
    message_data: *const u8,
    message_len: usize,
) -> Result<Envelope, LmResultCode> {
    // SAFETY：本函数继承调用方对句柄生命周期的保证。
    let Some(engine) = (unsafe { engine_ref(engine) }) else {
        return Err(LmResultCode::InvalidArgument);
    };
    if engine.shutdown.load(Ordering::Acquire) {
        return Err(LmResultCode::Shutdown);
    }
    if message_data.is_null() || message_len == 0 || message_len > MAX_PROTOCOL_MESSAGE_BYTES {
        return Err(LmResultCode::InvalidArgument);
    }
    // SAFETY：消息指针经过非空和长度上限验证，调用方保证本次同步调用期间可读。
    let message = unsafe { std::slice::from_raw_parts(message_data, message_len) };
    let envelope = Envelope::decode(message).map_err(|_| LmResultCode::MalformedMessage)?;
    envelope
        .validate_version()
        .map_err(|_| LmResultCode::ProtocolIncompatible)?;
    if envelope.operation_id.is_empty()
        || envelope.correlation_id.is_empty()
        || envelope.message_type.is_empty()
    {
        return Err(LmResultCode::MalformedMessage);
    }
    Ok(envelope)
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn ffi_guard(operation: impl FnOnce() -> LmResultCode) -> LmResultCode {
    catch_unwind(AssertUnwindSafe(operation)).unwrap_or(LmResultCode::Panic)
}

#[cfg(test)]
mod tests {
    use super::{
        LmBuffer, LmEngine, LmResultCode, MAX_FILE_LEASES, MAX_OUTSTANDING_BUFFERS,
        lm_engine_buffer_free, lm_engine_cancel, lm_engine_create, lm_engine_destroy,
        lm_engine_file_lease_create_desktop_path, lm_engine_file_lease_create_posix_descriptor,
        lm_engine_file_lease_create_temporary_path, lm_engine_file_lease_destroy,
        lm_engine_file_lease_expire, lm_engine_file_lease_is_active, lm_engine_file_lease_revoke,
        lm_engine_get_abi_version, lm_engine_get_compatibility, lm_engine_get_protocol_version,
        lm_engine_get_version, lm_engine_poll_event, lm_engine_send_host_response,
        lm_engine_shutdown, lm_engine_submit, lock_unpoisoned,
    };
    use linguamesh_domain::{FileLease, FileLeaseError, SecretValue};
    use linguamesh_protocol::{
        CompatibilitySnapshot, Envelope, HostSecretResponse, PROTOCOL_VERSION, SecretRequiredEvent,
        TextDeltaEvent, TranslateTextCommand, message_type,
    };
    use linguamesh_testkit::FakeProviderServer;
    use prost::Message;
    use std::ptr;
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn lifecycle_and_repeated_shutdown_are_safe() {
        let mut engine: *mut LmEngine = ptr::null_mut();
        // SAFETY：测试提供有效的输出指针并遵循句柄所有权协议。
        assert_eq!(
            unsafe { lm_engine_create(&raw mut engine) },
            LmResultCode::Ok
        );
        assert!(!engine.is_null());
        // SAFETY：句柄仍然有效且尚未销毁。
        assert_eq!(unsafe { lm_engine_cancel(engine) }, LmResultCode::Ok);
        // SAFETY：句柄仍然有效且尚未销毁。
        assert_eq!(unsafe { lm_engine_shutdown(engine) }, LmResultCode::Ok);
        // SAFETY：重复关闭只设置幂等状态。
        assert_eq!(unsafe { lm_engine_shutdown(engine) }, LmResultCode::Ok);
        // SAFETY：句柄由本测试创建且只销毁一次。
        assert_eq!(unsafe { lm_engine_destroy(engine) }, LmResultCode::Ok);
    }

    #[test]
    fn invalid_inputs_are_rejected() {
        // SAFETY：空指针用于验证边界拒绝路径，不会被解引用。
        assert_eq!(
            unsafe { lm_engine_create(ptr::null_mut()) },
            LmResultCode::InvalidArgument
        );
        let mut output = LmBuffer {
            data: ptr::null_mut(),
            len: 0,
            capacity: 0,
            allocation_id: 0,
        };
        // SAFETY：空引擎句柄用于验证边界拒绝路径。
        assert_eq!(
            unsafe { lm_engine_poll_event(ptr::null_mut(), 0, &raw mut output) },
            LmResultCode::InvalidArgument
        );
        // SAFETY：空引擎句柄用于验证释放边界拒绝路径。
        assert_eq!(
            unsafe { lm_engine_buffer_free(ptr::null_mut(), &raw mut output) },
            LmResultCode::InvalidArgument
        );
    }

    #[test]
    fn buffer_release_is_engine_scoped_and_rejects_invalid_descriptors() {
        let mut first_engine: *mut LmEngine = ptr::null_mut();
        let mut second_engine: *mut LmEngine = ptr::null_mut();
        // SAFETY：测试提供有效的输出指针并遵循句柄所有权协议。
        assert_eq!(
            unsafe { lm_engine_create(&raw mut first_engine) },
            LmResultCode::Ok
        );
        // SAFETY：测试提供有效的输出指针并遵循句柄所有权协议。
        assert_eq!(
            unsafe { lm_engine_create(&raw mut second_engine) },
            LmResultCode::Ok
        );
        let mut client_owned = vec![7_u8];
        let mut forged = LmBuffer {
            data: client_owned.as_mut_ptr(),
            len: client_owned.len(),
            capacity: client_owned.capacity(),
            allocation_id: u64::MAX,
        };
        // SAFETY：测试使用有效但不属于核心的指针验证拒绝路径。
        assert_eq!(
            unsafe { lm_engine_buffer_free(first_engine, &raw mut forged) },
            LmResultCode::InvalidArgument
        );
        assert_eq!(client_owned, vec![7]);

        let mut first = LmBuffer {
            data: ptr::null_mut(),
            len: 0,
            capacity: 0,
            allocation_id: 0,
        };
        let mut second = LmBuffer {
            data: ptr::null_mut(),
            len: 0,
            capacity: 0,
            allocation_id: 0,
        };
        // SAFETY：两个句柄都有效，且测试仅借用其内部辅助方法。
        assert_eq!(
            unsafe { write_test_buffer(first_engine, &mut first, vec![1, 2, 3]) },
            LmResultCode::Ok
        );
        // SAFETY：两个句柄都有效，且测试仅借用其内部辅助方法。
        assert_eq!(
            unsafe { write_test_buffer(second_engine, &mut second, vec![4, 5, 6]) },
            LmResultCode::Ok
        );
        assert_eq!(first.allocation_id, second.allocation_id);
        let mut duplicate = LmBuffer {
            data: first.data,
            len: first.len,
            capacity: first.capacity,
            allocation_id: first.allocation_id,
        };
        // SAFETY：描述符属于另一个引擎，用于验证所有权隔离。
        assert_eq!(
            unsafe { lm_engine_buffer_free(second_engine, &raw mut duplicate) },
            LmResultCode::InvalidArgument
        );
        // SAFETY：原始描述符来自第一个引擎且仅在此处释放一次。
        assert_eq!(
            unsafe { lm_engine_buffer_free(first_engine, &raw mut first) },
            LmResultCode::Ok
        );
        let mut replacement = LmBuffer {
            data: ptr::null_mut(),
            len: 0,
            capacity: 0,
            allocation_id: 0,
        };
        // SAFETY：句柄有效，且测试仅借用其内部辅助方法。
        assert_eq!(
            unsafe { write_test_buffer(first_engine, &mut replacement, vec![7, 8, 9]) },
            LmResultCode::Ok
        );
        assert_ne!(duplicate.allocation_id, replacement.allocation_id);
        // SAFETY：复制描述符用于验证地址复用后仍拒绝陈旧令牌。
        assert_eq!(
            unsafe { lm_engine_buffer_free(first_engine, &raw mut duplicate) },
            LmResultCode::InvalidArgument
        );
        // SAFETY：关闭句柄后仍允许释放该句柄拥有的活动缓冲区。
        assert_eq!(
            unsafe { lm_engine_shutdown(first_engine) },
            LmResultCode::Ok
        );
        // SAFETY：替代描述符来自第一个引擎且仅在此处释放一次。
        assert_eq!(
            unsafe { lm_engine_buffer_free(first_engine, &raw mut replacement) },
            LmResultCode::Ok
        );
        // SAFETY：已清空的原始描述符允许幂等释放。
        assert_eq!(
            unsafe { lm_engine_buffer_free(first_engine, &raw mut first) },
            LmResultCode::Ok
        );
        // SAFETY：第二个描述符由第二个引擎分配且只释放一次。
        assert_eq!(
            unsafe { lm_engine_buffer_free(second_engine, &raw mut second) },
            LmResultCode::Ok
        );
        // SAFETY：两个句柄均由本测试创建且各销毁一次。
        assert_eq!(unsafe { lm_engine_destroy(first_engine) }, LmResultCode::Ok);
        // SAFETY：两个句柄均由本测试创建且各销毁一次。
        assert_eq!(
            unsafe { lm_engine_destroy(second_engine) },
            LmResultCode::Ok
        );
    }

    #[test]
    fn outstanding_buffer_limit_is_enforced() {
        let mut engine: *mut LmEngine = ptr::null_mut();
        // SAFETY：测试提供有效的输出指针并遵循句柄所有权协议。
        assert_eq!(
            unsafe { lm_engine_create(&raw mut engine) },
            LmResultCode::Ok
        );
        let mut buffers = Vec::with_capacity(MAX_OUTSTANDING_BUFFERS);
        for index in 0..MAX_OUTSTANDING_BUFFERS {
            let mut buffer = LmBuffer {
                data: ptr::null_mut(),
                len: 0,
                capacity: 0,
                allocation_id: 0,
            };
            let byte = u8::try_from(index).expect("bounded buffer index");
            // SAFETY：句柄有效，输出描述符可写，且每次调用仅创建一个活动分配。
            assert_eq!(
                unsafe { write_test_buffer(engine, &mut buffer, vec![byte]) },
                LmResultCode::Ok
            );
            buffers.push(buffer);
        }
        // SAFETY：句柄有效，测试只读取受互斥锁保护的分配数量。
        assert_eq!(
            lock_unpoisoned(&unsafe { &*engine }.buffer_allocations).len(),
            MAX_OUTSTANDING_BUFFERS
        );
        let mut overflow = LmBuffer {
            data: ptr::null_mut(),
            len: 0,
            capacity: 0,
            allocation_id: 0,
        };
        // SAFETY：句柄有效，输出描述符可写，轮询在读取事件前拒绝超限请求。
        assert_eq!(
            unsafe { lm_engine_poll_event(engine, 0, &raw mut overflow) },
            LmResultCode::ResourceExhausted
        );
        assert!(overflow.data.is_null());
        assert_eq!(overflow.allocation_id, 0);
        let mut released = buffers.pop().expect("buffer at limit");
        // SAFETY：描述符属于该引擎且仅释放一次。
        assert_eq!(
            unsafe { lm_engine_buffer_free(engine, &raw mut released) },
            LmResultCode::Ok
        );
        // SAFETY：释放一个槽位后，空轮询可以预留并归还该槽位。
        assert_eq!(
            unsafe { lm_engine_poll_event(engine, 0, &raw mut overflow) },
            LmResultCode::Ok
        );
        let mut replacement = LmBuffer {
            data: ptr::null_mut(),
            len: 0,
            capacity: 0,
            allocation_id: 0,
        };
        // SAFETY：释放槽位后允许恰好一个新活动分配。
        assert_eq!(
            unsafe { write_test_buffer(engine, &mut replacement, vec![u8::MAX]) },
            LmResultCode::Ok
        );
        buffers.push(replacement);
        // SAFETY：句柄有效，测试只读取受互斥锁保护的分配数量。
        assert_eq!(
            lock_unpoisoned(&unsafe { &*engine }.buffer_allocations).len(),
            MAX_OUTSTANDING_BUFFERS
        );
        for mut buffer in buffers {
            // SAFETY：每个描述符都属于该引擎且仅释放一次。
            assert_eq!(
                unsafe { lm_engine_buffer_free(engine, &raw mut buffer) },
                LmResultCode::Ok
            );
        }
        // SAFETY：句柄由本测试创建且只销毁一次。
        assert_eq!(unsafe { lm_engine_destroy(engine) }, LmResultCode::Ok);
    }

    #[test]
    fn concurrent_poll_timeout_includes_receiver_lock_wait() {
        let mut engine: *mut LmEngine = ptr::null_mut();
        // SAFETY：测试提供有效的输出指针并遵循句柄所有权协议。
        assert_eq!(
            unsafe { lm_engine_create(&raw mut engine) },
            LmResultCode::Ok
        );
        let engine_address = engine as usize;
        let first_poll = thread::spawn(move || {
            let mut output = LmBuffer {
                data: ptr::null_mut(),
                len: 0,
                capacity: 0,
                allocation_id: 0,
            };
            // SAFETY：所有者在线程结束前保持句柄有效，输出描述符可写。
            let result = unsafe {
                lm_engine_poll_event(engine_address as *mut LmEngine, 200, &raw mut output)
            };
            assert_eq!(result, LmResultCode::Ok);
            assert!(output.data.is_null());
        });
        thread::sleep(Duration::from_millis(25));
        let mut output = LmBuffer {
            data: ptr::null_mut(),
            len: 0,
            capacity: 0,
            allocation_id: 0,
        };
        let started = Instant::now();
        // SAFETY：句柄仍然有效，输出描述符可写，且超时有界。
        assert_eq!(
            unsafe { lm_engine_poll_event(engine, 10, &raw mut output) },
            LmResultCode::Ok
        );
        assert!(started.elapsed() < Duration::from_millis(100));
        first_poll.join().expect("first poll");
        // SAFETY：句柄由本测试创建且只销毁一次。
        assert_eq!(unsafe { lm_engine_destroy(engine) }, LmResultCode::Ok);
    }

    #[test]
    fn concurrent_control_calls_are_serialized_and_fail_closed_after_shutdown() {
        let mut engine: *mut LmEngine = ptr::null_mut();
        // SAFETY：测试提供有效的输出指针并遵循句柄所有权协议。
        assert_eq!(
            unsafe { lm_engine_create(&raw mut engine) },
            LmResultCode::Ok
        );
        let engine_address = engine as usize;
        let workers = (0..12)
            .map(|index| {
                thread::spawn(move || {
                    let engine = engine_address as *mut LmEngine;
                    match index % 3 {
                        0 => {
                            // SAFETY：所有线程只调用受内部同步保护的控制接口，句柄在 join 前保持有效。
                            let result = unsafe { lm_engine_cancel(engine) };
                            assert!(matches!(result, LmResultCode::Ok | LmResultCode::Shutdown));
                        }
                        1 => {
                            // SAFETY：重复关闭是公开的幂等操作，句柄在 join 前保持有效。
                            assert_eq!(unsafe { lm_engine_shutdown(engine) }, LmResultCode::Ok);
                        }
                        _ => {
                            let mut output = LmBuffer {
                                data: ptr::null_mut(),
                                len: 0,
                                capacity: 0,
                                allocation_id: 0,
                            };
                            // SAFETY：输出描述符仅由当前线程访问，句柄在 join 前保持有效。
                            let result =
                                unsafe { lm_engine_poll_event(engine, 10, &raw mut output) };
                            assert!(matches!(result, LmResultCode::Ok | LmResultCode::Shutdown));
                            if !output.data.is_null() {
                                // SAFETY：轮询返回的缓冲区属于同一引擎并在当前线程释放一次。
                                assert_eq!(
                                    unsafe { lm_engine_buffer_free(engine, &raw mut output) },
                                    LmResultCode::Ok
                                );
                            }
                        }
                    }
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().expect("concurrent FFI control call");
        }

        let mut output = LmBuffer {
            data: ptr::null_mut(),
            len: 0,
            capacity: 0,
            allocation_id: 0,
        };
        // SAFETY：所有并发调用均已结束，句柄仍由本测试独占并可安全销毁。
        assert_eq!(
            unsafe { lm_engine_poll_event(engine, 0, &raw mut output) },
            LmResultCode::Shutdown
        );
        // SAFETY：句柄由本测试创建且只销毁一次。
        assert_eq!(unsafe { lm_engine_destroy(engine) }, LmResultCode::Ok);
    }

    #[test]
    fn exported_versions_match_contract_constants() {
        assert_eq!(lm_engine_get_abi_version(), 1);
        assert_eq!(lm_engine_get_protocol_version(), PROTOCOL_VERSION);
        assert_eq!(lm_engine_get_version(), PROTOCOL_VERSION);
    }

    #[test]
    fn compatibility_snapshot_is_available_through_abi() {
        let mut engine: *mut LmEngine = ptr::null_mut();
        // SAFETY：测试提供有效的输出指针并遵循句柄所有权协议。
        assert_eq!(
            unsafe { lm_engine_create(&raw mut engine) },
            LmResultCode::Ok
        );
        let mut output = LmBuffer {
            data: ptr::null_mut(),
            len: 0,
            capacity: 0,
            allocation_id: 0,
        };
        // SAFETY：输出描述符为零初始化且句柄尚未销毁。
        assert_eq!(
            unsafe { lm_engine_get_compatibility(engine, &raw mut output) },
            LmResultCode::Ok
        );
        let bytes = unsafe { std::slice::from_raw_parts(output.data, output.len) }.to_vec();
        // SAFETY：缓冲区由本引擎分配且只释放一次。
        assert_eq!(
            unsafe { lm_engine_buffer_free(engine, &raw mut output) },
            LmResultCode::Ok
        );
        let envelope = Envelope::decode(bytes.as_slice()).expect("compatibility envelope");
        assert_eq!(envelope.message_type, message_type::COMPATIBILITY);
        assert!(!envelope.operation_id.is_empty());
        assert!(!envelope.correlation_id.is_empty());
        let snapshot = CompatibilitySnapshot::decode(envelope.payload.as_slice())
            .expect("compatibility snapshot");
        assert_eq!(snapshot.abi_major, 1);
        assert_eq!(snapshot.protocol_version, PROTOCOL_VERSION);
        assert_eq!(snapshot.provider_catalog_version, "0.1.0");
        assert!(
            snapshot
                .enabled_features
                .iter()
                .any(|feature| feature == "compatibility_negotiation_v1")
        );
        assert!(
            snapshot
                .enabled_features
                .iter()
                .any(|feature| feature == "file_lease_v1")
        );
        // SAFETY：句柄由本测试创建且只销毁一次。
        assert_eq!(unsafe { lm_engine_destroy(engine) }, LmResultCode::Ok);
    }

    #[test]
    fn ffi_file_lease_expiration_fails_closed_before_resource_access() {
        let lease = FileLease::temporary_path("/tmp/ffi-lease-input").expect("lease");
        let guard = lease.acquire().expect("borrow");
        lease.expire();
        assert_eq!(guard.resource(), Err(FileLeaseError::Expired));
        assert_eq!(
            lease.acquire().expect_err("expired borrow"),
            FileLeaseError::Expired
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn ffi_file_lease_registry_is_bounded_engine_scoped_and_reversible_only_by_destroy() {
        let mut first_engine: *mut LmEngine = ptr::null_mut();
        let mut second_engine: *mut LmEngine = ptr::null_mut();
        // SAFETY：测试提供有效的输出指针并遵循句柄所有权协议。
        assert_eq!(
            unsafe { lm_engine_create(&raw mut first_engine) },
            LmResultCode::Ok
        );
        // SAFETY：测试提供有效的输出指针并遵循句柄所有权协议。
        assert_eq!(
            unsafe { lm_engine_create(&raw mut second_engine) },
            LmResultCode::Ok
        );

        let path = b"/tmp/ffi-file-lease";
        let mut lease_id = 0_u64;
        // SAFETY：路径切片在调用期间保持有效，输出令牌槽位可写且为零。
        assert_eq!(
            unsafe {
                lm_engine_file_lease_create_desktop_path(
                    first_engine,
                    path.as_ptr(),
                    path.len(),
                    &raw mut lease_id,
                )
            },
            LmResultCode::Ok
        );
        assert_ne!(lease_id, 0);
        let mut active = 0_u8;
        // SAFETY：句柄和输出状态槽位均由本测试独占。
        assert_eq!(
            unsafe { lm_engine_file_lease_is_active(first_engine, lease_id, &raw mut active) },
            LmResultCode::Ok
        );
        assert_eq!(active, 1);
        // SAFETY：句柄和租约令牌来自同一引擎。
        assert_eq!(
            unsafe { lm_engine_file_lease_expire(first_engine, lease_id) },
            LmResultCode::Ok
        );
        // SAFETY：查询使用同一引擎和仍然存在的租约令牌。
        assert_eq!(
            unsafe { lm_engine_file_lease_is_active(first_engine, lease_id, &raw mut active) },
            LmResultCode::Ok
        );
        assert_eq!(active, 0);
        // SAFETY：错误引擎不得访问另一引擎的租约注册表。
        assert_eq!(
            unsafe { lm_engine_file_lease_is_active(second_engine, lease_id, &raw mut active) },
            LmResultCode::InvalidArgument
        );
        // SAFETY：撤销已经到期的租约保持幂等失效状态。
        assert_eq!(
            unsafe { lm_engine_file_lease_revoke(first_engine, lease_id) },
            LmResultCode::Ok
        );
        // SAFETY：租约令牌由本测试创建且只销毁一次。
        assert_eq!(
            unsafe { lm_engine_file_lease_destroy(first_engine, lease_id) },
            LmResultCode::Ok
        );
        // SAFETY：重复销毁必须被安全拒绝。
        assert_eq!(
            unsafe { lm_engine_file_lease_destroy(first_engine, lease_id) },
            LmResultCode::InvalidArgument
        );

        let mut leases = Vec::with_capacity(MAX_FILE_LEASES);
        for _ in 0..MAX_FILE_LEASES {
            let mut id = 0_u64;
            // SAFETY：路径切片和输出槽位在调用期间有效。
            assert_eq!(
                unsafe {
                    lm_engine_file_lease_create_temporary_path(
                        first_engine,
                        path.as_ptr(),
                        path.len(),
                        &raw mut id,
                    )
                },
                LmResultCode::Ok
            );
            leases.push(id);
        }
        let mut exhausted = 0_u64;
        // SAFETY：达到有界容量后验证创建失败且不消费新令牌。
        assert_eq!(
            unsafe {
                lm_engine_file_lease_create_temporary_path(
                    first_engine,
                    path.as_ptr(),
                    path.len(),
                    &raw mut exhausted,
                )
            },
            LmResultCode::ResourceExhausted
        );
        assert_eq!(exhausted, 0);
        let released = leases.remove(0);
        // SAFETY：释放测试中第一个有效租约。
        assert_eq!(
            unsafe { lm_engine_file_lease_destroy(first_engine, released) },
            LmResultCode::Ok
        );
        // SAFETY：释放容量后允许创建新的租约。
        assert_eq!(
            unsafe {
                lm_engine_file_lease_create_posix_descriptor(first_engine, 0, &raw mut exhausted)
            },
            LmResultCode::Ok
        );
        leases.push(exhausted);
        for id in leases {
            // SAFETY：每个令牌均由本测试创建且仍属于第一个引擎。
            assert_eq!(
                unsafe { lm_engine_file_lease_destroy(first_engine, id) },
                LmResultCode::Ok
            );
        }

        let mut shutdown_lease = 0_u64;
        // SAFETY：创建一个租约以验证关闭后的清理操作仍可用。
        assert_eq!(
            unsafe {
                lm_engine_file_lease_create_posix_descriptor(
                    first_engine,
                    0,
                    &raw mut shutdown_lease,
                )
            },
            LmResultCode::Ok
        );
        // SAFETY：关闭只停止新工作，不会使现有租约清理失效。
        assert_eq!(
            unsafe { lm_engine_shutdown(first_engine) },
            LmResultCode::Ok
        );
        // SAFETY：关闭后允许撤销并删除现有租约。
        assert_eq!(
            unsafe { lm_engine_file_lease_revoke(first_engine, shutdown_lease) },
            LmResultCode::Ok
        );
        // SAFETY：关闭后的租约销毁仍由所属引擎执行。
        assert_eq!(
            unsafe { lm_engine_file_lease_destroy(first_engine, shutdown_lease) },
            LmResultCode::Ok
        );
        let mut rejected = 0_u64;
        // SAFETY：关闭后的创建请求只验证安全拒绝结果。
        assert_eq!(
            unsafe {
                lm_engine_file_lease_create_posix_descriptor(first_engine, 0, &raw mut rejected)
            },
            LmResultCode::Shutdown
        );
        // SAFETY：两个句柄均由本测试创建且只销毁一次。
        assert_eq!(unsafe { lm_engine_destroy(first_engine) }, LmResultCode::Ok);
        assert_eq!(
            unsafe { lm_engine_destroy(second_engine) },
            LmResultCode::Ok
        );
    }

    #[test]
    fn submission_and_host_response_validate_protocol_envelopes() {
        let mut engine: *mut LmEngine = ptr::null_mut();
        // SAFETY：测试提供有效的输出指针并遵循句柄所有权协议。
        assert_eq!(
            unsafe { lm_engine_create(&raw mut engine) },
            LmResultCode::Ok
        );
        let response = encoded_envelope(PROTOCOL_VERSION, "host_response", Vec::new());
        // SAFETY：编码消息在调用期间保持有效且句柄尚未销毁。
        assert_eq!(
            unsafe { lm_engine_send_host_response(engine, response.as_ptr(), response.len()) },
            LmResultCode::UnsupportedMessage
        );
        let incompatible = encoded_envelope(PROTOCOL_VERSION + 1, "command", Vec::new());
        // SAFETY：编码消息在调用期间保持有效且句柄尚未销毁。
        assert_eq!(
            unsafe { lm_engine_submit(engine, incompatible.as_ptr(), incompatible.len()) },
            LmResultCode::ProtocolIncompatible
        );
        let unsupported = encoded_envelope(PROTOCOL_VERSION, "command", Vec::new());
        // SAFETY：编码消息在调用期间保持有效且句柄尚未销毁。
        assert_eq!(
            unsafe { lm_engine_submit(engine, unsupported.as_ptr(), unsupported.len()) },
            LmResultCode::UnsupportedMessage
        );
        let nonzero_sequence = Envelope {
            protocol_version: PROTOCOL_VERSION,
            operation_id: "operation".into(),
            correlation_id: "correlation".into(),
            sequence: 1,
            message_type: message_type::TRANSLATE_TEXT.into(),
            payload: TranslateTextCommand {
                endpoint: "http://127.0.0.1:8080/v1/".into(),
                model_id: "fake-translator".into(),
                source_text: "Hello".into(),
                target_locale: "zh-CN".into(),
                secret_ref: String::new(),
            }
            .encode_to_vec(),
        }
        .encode_to_vec();
        // SAFETY：编码消息在调用期间保持有效且句柄尚未销毁。
        assert_eq!(
            unsafe { lm_engine_submit(engine, nonzero_sequence.as_ptr(), nonzero_sequence.len(),) },
            LmResultCode::MalformedMessage
        );
        let malformed = [0xff];
        // SAFETY：字节数组在调用期间保持有效且句柄尚未销毁。
        assert_eq!(
            unsafe { lm_engine_submit(engine, malformed.as_ptr(), malformed.len()) },
            LmResultCode::MalformedMessage
        );
        // SAFETY：句柄由本测试创建且只销毁一次。
        assert_eq!(unsafe { lm_engine_destroy(engine) }, LmResultCode::Ok);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ffi_streams_real_fake_provider_events() {
        let server = FakeProviderServer::start().await.expect("server");
        let endpoint = server.base_url();
        let events = tokio::task::spawn_blocking(move || run_ffi_translation(&endpoint, false))
            .await
            .expect("native task");
        let output = events
            .iter()
            .filter(|event| event.message_type == message_type::TEXT_DELTA)
            .map(|event| {
                TextDeltaEvent::decode(event.payload.as_slice())
                    .expect("delta")
                    .text
            })
            .collect::<String>();
        assert_eq!(output, "你好，LinguaMesh！");
        assert!(
            events
                .iter()
                .any(|event| event.message_type == message_type::COMPLETED)
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| {
                    matches!(
                        event.message_type.as_str(),
                        message_type::COMPLETED | message_type::CANCELLED | message_type::FAILED
                    )
                })
                .count(),
            1
        );
        server.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ffi_cancellation_emits_one_cancelled_terminal() {
        let server = FakeProviderServer::start().await.expect("server");
        let endpoint = server.base_url();
        let events = tokio::task::spawn_blocking(move || run_ffi_translation(&endpoint, true))
            .await
            .expect("native task");
        assert!(
            events
                .iter()
                .any(|event| event.message_type == message_type::TEXT_DELTA)
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event.message_type == message_type::CANCELLED)
                .count(),
            1
        );
        assert!(
            events
                .iter()
                .all(|event| event.message_type != message_type::COMPLETED)
        );
        server.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ffi_host_secret_flow_emits_required_and_accepts_one_shot_response() {
        const SECRET_CANARY: &str = concat!("s", "k", "-LM_FFI_SECRET_CANARY_123456");
        let server =
            FakeProviderServer::start_requiring_bearer_token(SecretValue::new(SECRET_CANARY))
                .await
                .expect("server");
        let endpoint = server.base_url();
        let events = tokio::task::spawn_blocking(move || {
            run_ffi_authenticated_translation(&endpoint, SECRET_CANARY)
        })
        .await
        .expect("native task");
        let output = events
            .iter()
            .filter(|event| event.message_type == message_type::TEXT_DELTA)
            .map(|event| {
                TextDeltaEvent::decode(event.payload.as_slice())
                    .expect("delta")
                    .text
            })
            .collect::<String>();
        assert_eq!(output, "你好，LinguaMesh！");
        assert_eq!(
            events
                .iter()
                .filter(|event| event.message_type == message_type::SECRET_REQUIRED)
                .count(),
            1
        );
        assert!(
            events
                .iter()
                .any(|event| event.message_type == message_type::COMPLETED)
        );
        server.shutdown().await;
    }

    fn run_ffi_translation(endpoint: &str, cancel_after_delta: bool) -> Vec<Envelope> {
        let mut engine: *mut LmEngine = ptr::null_mut();
        // SAFETY：测试提供有效的输出指针并遵循句柄所有权协议。
        assert_eq!(
            unsafe { lm_engine_create(&raw mut engine) },
            LmResultCode::Ok
        );
        let model_id = if cancel_after_delta {
            "fake-slow-translator"
        } else {
            "fake-translator"
        };
        let command = TranslateTextCommand {
            endpoint: endpoint.into(),
            model_id: model_id.into(),
            source_text: "Hello".into(),
            target_locale: "zh-CN".into(),
            secret_ref: String::new(),
        };
        let envelope = encoded_envelope(
            PROTOCOL_VERSION,
            message_type::TRANSLATE_TEXT,
            command.encode_to_vec(),
        );
        // SAFETY：编码消息在调用期间保持有效且句柄尚未销毁。
        assert_eq!(
            unsafe { lm_engine_submit(engine, envelope.as_ptr(), envelope.len()) },
            LmResultCode::Ok
        );
        let mut events = Vec::new();
        for _ in 0..16 {
            let event = poll_envelope(engine);
            if event.message_type == message_type::TEXT_DELTA && cancel_after_delta {
                // SAFETY：句柄仍然有效且当前操作尚未终止。
                assert_eq!(unsafe { lm_engine_cancel(engine) }, LmResultCode::Ok);
            }
            let terminal = matches!(
                event.message_type.as_str(),
                message_type::COMPLETED | message_type::CANCELLED | message_type::FAILED
            );
            events.push(event);
            if terminal {
                break;
            }
        }
        // SAFETY：句柄由本测试创建且只销毁一次。
        assert_eq!(unsafe { lm_engine_destroy(engine) }, LmResultCode::Ok);
        events
    }

    fn run_ffi_authenticated_translation(endpoint: &str, secret: &str) -> Vec<Envelope> {
        let mut engine: *mut LmEngine = ptr::null_mut();
        // SAFETY：测试提供有效的输出指针并遵循句柄所有权协议。
        assert_eq!(
            unsafe { lm_engine_create(&raw mut engine) },
            LmResultCode::Ok
        );
        let command = TranslateTextCommand {
            endpoint: endpoint.into(),
            model_id: "fake-translator".into(),
            source_text: "Hello".into(),
            target_locale: "zh-CN".into(),
            secret_ref: "session:11111111-1111-4111-8111-111111111111".into(),
        };
        let envelope = encoded_envelope(
            PROTOCOL_VERSION,
            message_type::TRANSLATE_TEXT,
            command.encode_to_vec(),
        );
        // SAFETY：编码消息在调用期间保持有效且句柄尚未销毁。
        assert_eq!(
            unsafe { lm_engine_submit(engine, envelope.as_ptr(), envelope.len()) },
            LmResultCode::Ok
        );
        let mut events = Vec::new();
        for _ in 0..20 {
            let event = poll_envelope(engine);
            if event.message_type == message_type::SECRET_REQUIRED {
                let required =
                    SecretRequiredEvent::decode(event.payload.as_slice()).expect("secret required");
                assert_eq!(
                    required.secret_ref,
                    "session:11111111-1111-4111-8111-111111111111"
                );
                let response = Envelope {
                    protocol_version: PROTOCOL_VERSION,
                    operation_id: event.operation_id.clone(),
                    correlation_id: event.correlation_id.clone(),
                    sequence: 0,
                    message_type: message_type::HOST_SECRET_RESPONSE.into(),
                    payload: HostSecretResponse {
                        request_id: required.request_id,
                        resolution: "provided".into(),
                        secret: secret.into(),
                    }
                    .encode_to_vec(),
                }
                .encode_to_vec();
                let mut mismatched =
                    Envelope::decode(response.as_slice()).expect("response envelope");
                mismatched.correlation_id = "wrong-correlation".into();
                let mismatched = mismatched.encode_to_vec();
                // SAFETY：错误关联的响应必须被拒绝且不能消耗待处理请求。
                assert_eq!(
                    unsafe {
                        lm_engine_send_host_response(engine, mismatched.as_ptr(), mismatched.len())
                    },
                    LmResultCode::InvalidArgument
                );
                // SAFETY：响应缓冲区在调用期间保持有效且句柄尚未销毁。
                assert_eq!(
                    unsafe {
                        lm_engine_send_host_response(engine, response.as_ptr(), response.len())
                    },
                    LmResultCode::Ok
                );
                // SAFETY：一次性响应再次提交必须被拒绝。
                assert_eq!(
                    unsafe {
                        lm_engine_send_host_response(engine, response.as_ptr(), response.len())
                    },
                    LmResultCode::InvalidArgument
                );
            }
            let terminal = matches!(
                event.message_type.as_str(),
                message_type::COMPLETED | message_type::CANCELLED | message_type::FAILED
            );
            events.push(event);
            if terminal {
                break;
            }
        }
        // SAFETY：句柄由本测试创建且只销毁一次。
        assert_eq!(unsafe { lm_engine_destroy(engine) }, LmResultCode::Ok);
        events
    }

    fn poll_envelope(engine: *mut LmEngine) -> Envelope {
        let mut output = LmBuffer {
            data: ptr::null_mut(),
            len: 0,
            capacity: 0,
            allocation_id: 0,
        };
        // SAFETY：句柄有效，输出描述符可写，且超时有界。
        assert_eq!(
            unsafe { lm_engine_poll_event(engine, 2_000, &raw mut output) },
            LmResultCode::Ok
        );
        assert!(!output.data.is_null(), "event timeout");
        // SAFETY：缓冲区由本库返回，在释放前可读取 len 字节。
        let bytes = unsafe { std::slice::from_raw_parts(output.data, output.len) }.to_vec();
        // SAFETY：缓冲区由本库分配且只释放一次。
        assert_eq!(
            unsafe { lm_engine_buffer_free(engine, &raw mut output) },
            LmResultCode::Ok
        );
        Envelope::decode(bytes.as_slice()).expect("event envelope")
    }

    fn encoded_envelope(protocol_version: u32, message_type: &str, payload: Vec<u8>) -> Vec<u8> {
        Envelope {
            protocol_version,
            operation_id: "operation".into(),
            correlation_id: "correlation".into(),
            sequence: 0,
            message_type: message_type.into(),
            payload,
        }
        .encode_to_vec()
    }

    unsafe fn write_test_buffer(
        engine: *mut LmEngine,
        output: &mut LmBuffer,
        bytes: Vec<u8>,
    ) -> LmResultCode {
        // SAFETY：调用方保证句柄由本库创建且在本次调用期间有效。
        let engine = unsafe { &*engine };
        let Ok(buffer_slot) = engine.reserve_buffer_slot() else {
            return LmResultCode::ResourceExhausted;
        };
        engine.write_output_buffer(output, bytes, buffer_slot)
    }
}
