#![doc = "`LinguaMesh` 命令和事件的版本化线协议。"]

use prost::Message;

/// 当前命令和事件协议版本。
pub const PROTOCOL_VERSION: u32 = 1;

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
    use super::{Envelope, PROTOCOL_VERSION, ProtocolError};
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
}
