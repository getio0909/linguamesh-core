#![doc = "`LinguaMesh` 命令和事件的版本化线协议。"]

use prost::Message;

/// 当前命令和事件协议版本。
pub const PROTOCOL_VERSION: u32 = 1;

/// 定义稳定的消息类型标识。
pub mod message_type {
    /// 请求文本翻译。
    pub const TRANSLATE_TEXT: &str = "translate_text";
    /// 表示操作已经开始。
    pub const STARTED: &str = "started";
    /// 携带增量文本。
    pub const TEXT_DELTA: &str = "text_delta";
    /// 表示操作已经完成。
    pub const COMPLETED: &str = "completed";
    /// 表示操作已经取消。
    pub const CANCELLED: &str = "cancelled";
    /// 携带安全的失败信息。
    pub const FAILED: &str = "failed";
}

/// 包装跨原生边界传输的版本化消息。
#[derive(Clone, PartialEq, Message)]
pub struct Envelope {
    /// 协议版本。
    #[prost(uint32, tag = "1")]
    pub protocol_version: u32,
    /// 操作标识。
    #[prost(string, tag = "2")]
    pub operation_id: String,
    /// 关联标识。
    #[prost(string, tag = "3")]
    pub correlation_id: String,
    /// 单调递增的事件序号。
    #[prost(uint64, tag = "4")]
    pub sequence: u64,
    /// 稳定消息类型。
    #[prost(string, tag = "5")]
    pub message_type: String,
    /// 与消息类型对应的编码载荷。
    #[prost(bytes = "vec", tag = "6")]
    pub payload: Vec<u8>,
}

/// 包含启动文本翻译所需的非秘密参数。
#[derive(Clone, PartialEq, Message)]
pub struct TranslateTextCommand {
    /// 指向兼容提供商的基础端点。
    #[prost(string, tag = "1")]
    pub endpoint: String,
    /// 明确选择的模型标识。
    #[prost(string, tag = "2")]
    pub model_id: String,
    /// 作为非可信数据处理的源文本。
    #[prost(string, tag = "3")]
    pub source_text: String,
    /// 目标 BCP 47 语言标签。
    #[prost(string, tag = "4")]
    pub target_locale: String,
}

/// 携带一段增量翻译文本。
#[derive(Clone, PartialEq, Message)]
pub struct TextDeltaEvent {
    /// 新增文本而不是累计文本。
    #[prost(string, tag = "1")]
    pub text: String,
}

/// 携带可本地化分类和安全英文消息。
#[derive(Clone, PartialEq, Message)]
pub struct FailureEvent {
    /// 稳定的蛇形错误类别。
    #[prost(string, tag = "1")]
    pub error_kind: String,
    /// 不包含秘密的英文诊断。
    #[prost(string, tag = "2")]
    pub message: String,
}

impl Envelope {
    /// 验证调用方能否处理该消息。
    pub fn validate_version(&self) -> Result<(), ProtocolError> {
        if self.protocol_version == PROTOCOL_VERSION {
            Ok(())
        } else {
            Err(ProtocolError::IncompatibleVersion {
                expected: PROTOCOL_VERSION,
                actual: self.protocol_version,
            })
        }
    }
}

/// 描述协议验证失败。
#[derive(Debug, Eq, PartialEq)]
pub enum ProtocolError {
    /// 消息使用了不兼容版本。
    IncompatibleVersion {
        /// 当前实现要求的版本。
        expected: u32,
        /// 消息提供的版本。
        actual: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::{
        Envelope, PROTOCOL_VERSION, ProtocolError, TextDeltaEvent, TranslateTextCommand,
        message_type,
    };
    use prost::Message;

    #[test]
    fn envelope_round_trips_and_validates() {
        let envelope = Envelope {
            protocol_version: PROTOCOL_VERSION,
            operation_id: "operation".into(),
            correlation_id: "correlation".into(),
            sequence: 3,
            message_type: "text_delta".into(),
            payload: b"payload".to_vec(),
        };
        let decoded = Envelope::decode(envelope.encode_to_vec().as_slice()).expect("decode");
        assert_eq!(decoded, envelope);
        assert_eq!(decoded.validate_version(), Ok(()));
    }

    #[test]
    fn incompatible_version_is_rejected() {
        let envelope = Envelope {
            protocol_version: PROTOCOL_VERSION + 1,
            operation_id: String::new(),
            correlation_id: String::new(),
            sequence: 0,
            message_type: String::new(),
            payload: Vec::new(),
        };
        assert_eq!(
            envelope.validate_version(),
            Err(ProtocolError::IncompatibleVersion {
                expected: PROTOCOL_VERSION,
                actual: PROTOCOL_VERSION + 1,
            })
        );
    }

    #[test]
    fn native_command_and_event_payloads_round_trip() {
        let command = TranslateTextCommand {
            endpoint: "http://127.0.0.1:8080/v1/".into(),
            model_id: "fake-translator".into(),
            source_text: "Hello".into(),
            target_locale: "zh-CN".into(),
        };
        let encoded_command = command.encode_to_vec();
        assert_eq!(
            TranslateTextCommand::decode(encoded_command.as_slice()).expect("command"),
            command
        );
        let event = TextDeltaEvent {
            text: "你好".into(),
        };
        let envelope = Envelope {
            protocol_version: PROTOCOL_VERSION,
            operation_id: "operation".into(),
            correlation_id: "correlation".into(),
            sequence: 1,
            message_type: message_type::TEXT_DELTA.into(),
            payload: event.encode_to_vec(),
        };
        let decoded = Envelope::decode(envelope.encode_to_vec().as_slice()).expect("envelope");
        assert_eq!(decoded.message_type, message_type::TEXT_DELTA);
        assert_eq!(
            TextDeltaEvent::decode(decoded.payload.as_slice()).expect("event"),
            event
        );
    }
}
