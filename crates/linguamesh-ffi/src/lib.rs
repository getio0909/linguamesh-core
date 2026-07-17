#![doc = "`LinguaMesh` 的稳定 C ABI 基础。"]
#![allow(clippy::missing_safety_doc)]

use linguamesh_protocol::{Envelope, PROTOCOL_VERSION};
use prost::Message;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_PROTOCOL_MESSAGE_BYTES: usize = 1024 * 1024;

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
    /// 消息无法解码或缺少必要标识。
    MalformedMessage = 5,
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
}

/// 表示不向调用方暴露内部布局的引擎句柄。
pub struct LmEngine {
    shutdown: AtomicBool,
    cancelled: AtomicBool,
}

/// 创建新的不透明引擎句柄。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lm_engine_create(output: *mut *mut LmEngine) -> LmResultCode {
    ffi_guard(|| {
        if output.is_null() {
            return LmResultCode::InvalidArgument;
        }
        let engine = Box::new(LmEngine {
            shutdown: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
        });
        // SAFETY：调用方提供了经过非空验证且可写的输出指针。
        unsafe { output.write(Box::into_raw(engine)) };
        LmResultCode::Ok
    })
}

/// 返回当前命令和事件协议版本。
#[unsafe(no_mangle)]
pub const extern "C" fn lm_engine_get_version() -> u32 {
    PROTOCOL_VERSION
}

/// 验证并接收版本化命令消息。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lm_engine_submit(
    engine: *mut LmEngine,
    command_data: *const u8,
    command_len: usize,
) -> LmResultCode {
    ffi_guard(|| {
        // SAFETY：调用方保证句柄和消息缓冲区在本次同步调用期间有效。
        unsafe { validate_protocol_input(engine, command_data, command_len) }
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
        unsafe { validate_protocol_input(engine, response_data, response_len) }
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
        engine.cancelled.store(true, Ordering::Release);
        LmResultCode::Ok
    })
}

/// 以有界超时轮询下一条事件。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lm_engine_poll_event(
    engine: *mut LmEngine,
    _timeout_ms: u32,
    output: *mut LmBuffer,
) -> LmResultCode {
    ffi_guard(|| {
        // SAFETY：调用方保证非空句柄由本库创建且尚未销毁。
        let Some(engine) = (unsafe { engine_ref(engine) }) else {
            return LmResultCode::InvalidArgument;
        };
        if output.is_null() {
            return LmResultCode::InvalidArgument;
        }
        if engine.shutdown.load(Ordering::Acquire) {
            return LmResultCode::Shutdown;
        }
        // SAFETY：输出指针已经验证为非空，空缓冲区不拥有任何分配。
        unsafe {
            output.write(LmBuffer {
                data: ptr::null_mut(),
                len: 0,
                capacity: 0,
            });
        }
        LmResultCode::Ok
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
        unsafe { drop(Box::from_raw(engine)) };
        LmResultCode::Ok
    })
}

/// 释放 Rust 分配的缓冲区并清空描述符。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lm_buffer_free(buffer: *mut LmBuffer) -> LmResultCode {
    ffi_guard(|| {
        if buffer.is_null() {
            return LmResultCode::InvalidArgument;
        }
        // SAFETY：缓冲区描述符已经验证为非空并由调用方授予可写访问。
        let buffer = unsafe { &mut *buffer };
        if buffer.data.is_null() {
            if buffer.len == 0 && buffer.capacity == 0 {
                return LmResultCode::Ok;
            }
            return LmResultCode::InvalidArgument;
        }
        if buffer.len > buffer.capacity {
            return LmResultCode::InvalidArgument;
        }
        // SAFETY：数据、长度和容量必须来自本库转交给调用方的 Vec 分配。
        unsafe {
            drop(Vec::from_raw_parts(
                buffer.data,
                buffer.len,
                buffer.capacity,
            ));
        }
        buffer.data = ptr::null_mut();
        buffer.len = 0;
        buffer.capacity = 0;
        LmResultCode::Ok
    })
}

unsafe fn engine_ref<'a>(engine: *mut LmEngine) -> Option<&'a LmEngine> {
    if engine.is_null() {
        None
    } else {
        // SAFETY：非空句柄的生命周期由调用方保证覆盖本次同步调用。
        Some(unsafe { &*engine })
    }
}

unsafe fn validate_protocol_input(
    engine: *mut LmEngine,
    message_data: *const u8,
    message_len: usize,
) -> LmResultCode {
    // SAFETY：本函数继承调用方对句柄生命周期的保证。
    let Some(engine) = (unsafe { engine_ref(engine) }) else {
        return LmResultCode::InvalidArgument;
    };
    if engine.shutdown.load(Ordering::Acquire) {
        return LmResultCode::Shutdown;
    }
    if message_data.is_null() || message_len == 0 || message_len > MAX_PROTOCOL_MESSAGE_BYTES {
        return LmResultCode::InvalidArgument;
    }
    // SAFETY：消息指针经过非空和长度上限验证，调用方保证本次同步调用期间可读。
    let message = unsafe { std::slice::from_raw_parts(message_data, message_len) };
    let Ok(envelope) = Envelope::decode(message) else {
        return LmResultCode::MalformedMessage;
    };
    if envelope.validate_version().is_err() {
        return LmResultCode::ProtocolIncompatible;
    }
    if envelope.operation_id.is_empty()
        || envelope.correlation_id.is_empty()
        || envelope.message_type.is_empty()
    {
        return LmResultCode::MalformedMessage;
    }
    LmResultCode::Ok
}

fn ffi_guard(operation: impl FnOnce() -> LmResultCode) -> LmResultCode {
    catch_unwind(AssertUnwindSafe(operation)).unwrap_or(LmResultCode::Panic)
}

#[cfg(test)]
mod tests {
    use super::{
        LmBuffer, LmEngine, LmResultCode, lm_buffer_free, lm_engine_cancel, lm_engine_create,
        lm_engine_destroy, lm_engine_get_version, lm_engine_poll_event,
        lm_engine_send_host_response, lm_engine_shutdown, lm_engine_submit,
    };
    use linguamesh_protocol::{Envelope, PROTOCOL_VERSION};
    use prost::Message;
    use std::ptr;

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
        };
        // SAFETY：空引擎句柄用于验证边界拒绝路径。
        assert_eq!(
            unsafe { lm_engine_poll_event(ptr::null_mut(), 0, &raw mut output) },
            LmResultCode::InvalidArgument
        );
        // SAFETY：空缓冲区描述符表示没有分配需要释放。
        assert_eq!(unsafe { lm_buffer_free(&raw mut output) }, LmResultCode::Ok);
    }

    #[test]
    fn exported_version_matches_protocol() {
        assert_eq!(lm_engine_get_version(), PROTOCOL_VERSION);
    }

    #[test]
    fn submission_and_host_response_validate_protocol_envelopes() {
        let mut engine: *mut LmEngine = ptr::null_mut();
        // SAFETY：测试提供有效的输出指针并遵循句柄所有权协议。
        assert_eq!(
            unsafe { lm_engine_create(&raw mut engine) },
            LmResultCode::Ok
        );
        let valid = encoded_envelope(PROTOCOL_VERSION, "command");
        // SAFETY：编码消息在调用期间保持有效且句柄尚未销毁。
        assert_eq!(
            unsafe { lm_engine_submit(engine, valid.as_ptr(), valid.len()) },
            LmResultCode::Ok
        );
        let response = encoded_envelope(PROTOCOL_VERSION, "host_response");
        // SAFETY：编码消息在调用期间保持有效且句柄尚未销毁。
        assert_eq!(
            unsafe { lm_engine_send_host_response(engine, response.as_ptr(), response.len()) },
            LmResultCode::Ok
        );
        let incompatible = encoded_envelope(PROTOCOL_VERSION + 1, "command");
        // SAFETY：编码消息在调用期间保持有效且句柄尚未销毁。
        assert_eq!(
            unsafe { lm_engine_submit(engine, incompatible.as_ptr(), incompatible.len()) },
            LmResultCode::ProtocolIncompatible
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

    fn encoded_envelope(protocol_version: u32, message_type: &str) -> Vec<u8> {
        Envelope {
            protocol_version,
            operation_id: "operation".into(),
            correlation_id: "correlation".into(),
            sequence: 0,
            message_type: message_type.into(),
            payload: Vec::new(),
        }
        .encode_to_vec()
    }
}
