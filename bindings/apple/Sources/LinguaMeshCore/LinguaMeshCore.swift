import CLinguaMeshCore
import Foundation

/// 包含已加载核心的边界版本。
public struct CoreCompatibility: Equatable, Sendable {
    public let abiMajor: UInt32
    public let protocolVersion: UInt32
}

/// 镜像稳定 C ABI 结果码。
public enum CoreResult: Int32, Sendable {
    case ok = 0
    case invalidArgument = 1
    case shutdown = 2
    case panic = 3
    case protocolIncompatible = 4
    case malformedMessage = 5
    case busy = 6
    case unsupportedMessage = 7
    case `internal` = 8
    case resourceExhausted = 9
}

/// 表示核心 ABI 调用失败。
public struct CoreError: Error, Equatable, Sendable, LocalizedError {
    public let rawValue: Int32

    public var errorDescription: String? {
        switch CoreResult(rawValue: rawValue) {
        case .ok:
            return "LinguaMesh call succeeded."
        case .invalidArgument:
            return "LinguaMesh rejected an invalid argument."
        case .shutdown:
            return "LinguaMesh engine is shut down."
        case .panic:
            return "LinguaMesh contained an internal panic."
        case .protocolIncompatible:
            return "LinguaMesh protocol version is incompatible."
        case .malformedMessage:
            return "LinguaMesh rejected a malformed message."
        case .busy:
            return "LinguaMesh engine already has an active operation."
        case .unsupportedMessage:
            return "LinguaMesh does not support this message type."
        case .internal:
            return "LinguaMesh could not initialize an internal resource."
        case .resourceExhausted:
            return "LinguaMesh engine has too many outstanding buffers."
        case nil:
            return "LinguaMesh returned an unknown result code."
        }
    }
}

/// 表示已加载核心与客户端合同不兼容。
public struct CoreCompatibilityError: Error, Equatable, Sendable, LocalizedError {
    public let expected: CoreCompatibility
    public let actual: CoreCompatibility

    public var errorDescription: String? {
        "Loaded LinguaMesh core is incompatible."
    }
}

/// 独占持有一个核心句柄并防止关闭与进行中调用竞争。
public final class LinguaMeshEngine: @unchecked Sendable {
    public static let abiVersionMajor: UInt32 = 1
    public static let protocolVersion: UInt32 = 1

    public let compatibility: CoreCompatibility

    private let lifecycle = NSCondition()
    private var handle: OpaquePointer?
    private var activeCalls = 0
    private var closing = false

    /// 验证 ABI 和协议后创建引擎。
    public init(
        expectedABI: UInt32 = abiVersionMajor,
        expectedProtocol: UInt32 = protocolVersion
    ) throws {
        let actual = Self.queryCompatibility()
        let expected = CoreCompatibility(
            abiMajor: expectedABI,
            protocolVersion: expectedProtocol
        )
        guard actual == expected else {
            throw CoreCompatibilityError(expected: expected, actual: actual)
        }
        var created: OpaquePointer?
        try check(lm_engine_create(&created))
        guard let created else {
            throw CoreError(rawValue: Int32(LM_RESULT_INTERNAL))
        }
        handle = created
        compatibility = actual
    }

    deinit {
        closeWithoutThrowing()
    }

    /// 返回已加载核心的 ABI 和协议版本。
    public static func queryCompatibility() -> CoreCompatibility {
        CoreCompatibility(
            abiMajor: lm_engine_get_abi_version(),
            protocolVersion: lm_engine_get_protocol_version()
        )
    }

    /// 提交已编码的版本化 Protobuf 命令封套。
    public func submit(_ command: Data) throws {
        try withHandle { current in
            try command.withUnsafeBytes { bytes in
                let pointer = bytes.bindMemory(to: UInt8.self).baseAddress
                try check(lm_engine_submit(current, pointer, bytes.count))
            }
        }
    }

    /// 在有界超时内轮询下一条原始事件，超时时返回空数据。
    public func pollEvent(timeoutMilliseconds: UInt32) throws -> Data {
        try withHandle { current in
            var buffer = LmBuffer()
            try check(lm_engine_poll_event(current, timeoutMilliseconds, &buffer))
            let copied: Data
            if let data = buffer.data, buffer.len > 0 {
                copied = Data(bytes: data, count: buffer.len)
            } else {
                copied = Data()
            }
            try check(lm_engine_buffer_free(current, &buffer))
            return copied
        }
    }

    /// 发送已编码的版本化主机响应封套。
    public func sendHostResponse(_ response: Data) throws {
        try withHandle { current in
            try response.withUnsafeBytes { bytes in
                let pointer = bytes.bindMemory(to: UInt8.self).baseAddress
                try check(lm_engine_send_host_response(current, pointer, bytes.count))
            }
        }
    }

    /// 请求当前操作取消且不重试。
    public func cancel() throws {
        try withHandle { current in
            try check(lm_engine_cancel(current))
        }
    }

    /// 停止接受新工作并取消活动操作。
    public func shutdown() throws {
        try withHandle { current in
            try check(lm_engine_shutdown(current))
        }
    }

    /// 等待已进入 ABI 的调用返回后销毁句柄。
    public func close() throws {
        let current = detachHandle()
        guard let current else {
            return
        }
        let shutdownResult = lm_engine_shutdown(current)
        let destroyResult = lm_engine_destroy(current)
        try check(shutdownResult)
        try check(destroyResult)
    }

    private func withHandle<T>(_ body: (OpaquePointer) throws -> T) throws -> T {
        lifecycle.lock()
        guard !closing, let current = handle else {
            lifecycle.unlock()
            throw CoreError(rawValue: Int32(LM_RESULT_SHUTDOWN))
        }
        activeCalls += 1
        lifecycle.unlock()
        defer {
            lifecycle.lock()
            activeCalls -= 1
            if activeCalls == 0 {
                lifecycle.broadcast()
            }
            lifecycle.unlock()
        }
        return try body(current)
    }

    private func detachHandle() -> OpaquePointer? {
        lifecycle.lock()
        if closing {
            lifecycle.unlock()
            return nil
        }
        closing = true
        while activeCalls > 0 {
            lifecycle.wait()
        }
        let current = handle
        handle = nil
        lifecycle.unlock()
        return current
    }

    private func closeWithoutThrowing() {
        guard let current = detachHandle() else {
            return
        }
        _ = lm_engine_shutdown(current)
        _ = lm_engine_destroy(current)
    }
}

private func check(_ result: LmResultCode) throws {
    let rawValue = Int32(result)
    guard rawValue == Int32(LM_RESULT_OK) else {
        throw CoreError(rawValue: rawValue)
    }
}
