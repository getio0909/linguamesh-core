package org.linguamesh.core

import com.google.protobuf.ByteString
import com.google.protobuf.InvalidProtocolBufferException
import org.linguamesh.core.protocol.Envelope
import org.linguamesh.core.protocol.HostSecretResponse
import org.linguamesh.core.protocol.TranslateTextCommand
import java.io.Closeable
import java.util.concurrent.locks.ReentrantReadWriteLock
import kotlin.concurrent.read
import kotlin.concurrent.write

/** 包含已加载核心的边界版本。 */
data class CoreCompatibility(
    val abiMajor: UInt,
    val protocolVersion: UInt,
)

/** 镜像稳定 C ABI 结果码。 */
enum class CoreResult(val rawValue: Int) {
    OK(0),
    INVALID_ARGUMENT(1),
    SHUTDOWN(2),
    PANIC(3),
    PROTOCOL_INCOMPATIBLE(4),
    MALFORMED_MESSAGE(5),
    BUSY(6),
    UNSUPPORTED_MESSAGE(7),
    INTERNAL(8),
    RESOURCE_EXHAUSTED(9),
    UNKNOWN(Int.MIN_VALUE),
    ;

    companion object {
        /** 从原始 ABI 值创建稳定结果。 */
        fun fromRawValue(value: Int): CoreResult = entries.firstOrNull { it.rawValue == value } ?: UNKNOWN
    }
}

/** 表示宿主对一次性秘密请求的有界处理结果。 */
enum class HostSecretResolution(val wireValue: String) {
    PROVIDED("provided"),
    UNAVAILABLE("unavailable"),
    SECURE_STORAGE_UNAVAILABLE("secure_storage_unavailable"),
}

/** 表示核心 ABI 调用失败。 */
class CoreException(
    val result: CoreResult,
    cause: Throwable? = null,
) : IllegalStateException(messageFor(result), cause) {
    companion object {
        private fun messageFor(result: CoreResult): String = when (result) {
            CoreResult.OK -> "LinguaMesh call succeeded."
            CoreResult.INVALID_ARGUMENT -> "LinguaMesh rejected an invalid argument."
            CoreResult.SHUTDOWN -> "LinguaMesh engine is shut down."
            CoreResult.PANIC -> "LinguaMesh contained an internal panic."
            CoreResult.PROTOCOL_INCOMPATIBLE -> "LinguaMesh protocol version is incompatible."
            CoreResult.MALFORMED_MESSAGE -> "LinguaMesh rejected a malformed message."
            CoreResult.BUSY -> "LinguaMesh engine already has an active operation."
            CoreResult.UNSUPPORTED_MESSAGE -> "LinguaMesh does not support this message type."
            CoreResult.INTERNAL -> "LinguaMesh could not initialize an internal resource."
            CoreResult.RESOURCE_EXHAUSTED -> "LinguaMesh engine has too many outstanding buffers."
            CoreResult.UNKNOWN -> "LinguaMesh returned an unknown result code."
        }
    }
}

/** 表示已加载核心与客户端合同不兼容。 */
class CoreCompatibilityException(
    val expected: CoreCompatibility,
    val actual: CoreCompatibility,
) : IllegalStateException("Loaded LinguaMesh core is incompatible.")

/** 独占持有一个核心句柄并隔离所有原始 JNI 调用。 */
class LinguaMeshEngine private constructor(
    private var handle: Long,
    val compatibility: CoreCompatibility,
) : Closeable {
    private val lifecycleLock = ReentrantReadWriteLock()

    /** 提交已编码的版本化 Protobuf 命令封套。 */
    fun submit(command: ByteArray) = withHandle { current ->
        checkResult(NativeBridge.submit(current, command))
    }

    /** 提交已构造的 Protobuf 命令封套。 */
    fun submit(command: Envelope) {
        require(command.protocolVersion.toUInt() == compatibility.protocolVersion) {
            "Command protocol version does not match the loaded core."
        }
        submit(command.toByteArray())
    }

    /** 为通用 OpenAI 兼容端点提交文本翻译。 */
    fun translateText(
        operationId: String,
        correlationId: String,
        endpoint: String,
        modelId: String,
        sourceText: String,
        targetLocale: String,
        organization: String? = null,
        project: String? = null,
        customHeadersJson: String? = null,
        secretRef: String? = null,
    ) {
        val payload = TranslateTextCommand.newBuilder()
            .setEndpoint(endpoint)
            .setModelId(modelId)
            .setSourceText(sourceText)
            .setTargetLocale(targetLocale)
            .apply {
                secretRef?.let(::setSecretRef)
                organization?.let(::setOrganization)
                project?.let(::setProject)
                customHeadersJson?.let(::setCustomHeadersJson)
            }
            .build()
        val envelope = Envelope.newBuilder()
            .setProtocolVersion(compatibility.protocolVersion.toInt())
            .setOperationId(operationId)
            .setCorrelationId(correlationId)
            .setSequence(0)
            .setMessageType(MESSAGE_TRANSLATE_TEXT)
            .setPayload(ByteString.copyFrom(payload.toByteArray()))
            .build()
        submit(envelope)
    }

    /** 在有界超时内轮询下一条原始事件，超时时返回空数组。 */
    fun pollEvent(timeoutMillis: UInt): ByteArray = withHandle { current ->
        val output = arrayOfNulls<ByteArray>(1)
        checkResult(NativeBridge.pollEvent(current, timeoutMillis.toInt(), output))
        output[0] ?: ByteArray(0)
    }

    /** 在有界超时内轮询并解码下一条事件封套。 */
    fun pollEnvelope(timeoutMillis: UInt): Envelope? {
        val bytes = pollEvent(timeoutMillis)
        if (bytes.isEmpty()) {
            return null
        }
        val envelope = try {
            Envelope.parseFrom(bytes)
        } catch (cause: InvalidProtocolBufferException) {
            throw CoreException(CoreResult.MALFORMED_MESSAGE, cause)
        }
        if (envelope.protocolVersion.toUInt() != compatibility.protocolVersion) {
            throw CoreCompatibilityException(
                expected = compatibility,
                actual = compatibility.copy(protocolVersion = envelope.protocolVersion.toUInt()),
            )
        }
        return envelope
    }

    /** 在有界超时内轮询公共事件，且不向调用方暴露 Protobuf 类型。 */
    fun pollDecodedEvent(timeoutMillis: UInt): CoreEvent? {
        val envelope = pollEnvelope(timeoutMillis) ?: return null
        return decodeCoreEvent(envelope)
    }

    /** 发送已编码的版本化主机响应封套。 */
    fun sendHostResponse(response: ByteArray) = withHandle { current ->
        checkResult(NativeBridge.sendHostResponse(current, response))
    }

    /** 编码并发送一次性主机秘密响应，避免应用直接依赖 Protobuf 类型。 */
    fun sendHostResponse(
        operationId: String,
        correlationId: String,
        requestId: String,
        resolution: HostSecretResolution,
        secret: String? = null,
    ) {
        require(operationId.isNotBlank()) { "Operation id must not be blank." }
        require(correlationId.isNotBlank()) { "Correlation id must not be blank." }
        require(requestId.isNotBlank() && requestId.length <= MAX_HOST_REQUEST_ID_LENGTH) {
            "Host secret request id is invalid."
        }
        val value = secret.orEmpty()
        when (resolution) {
            HostSecretResolution.PROVIDED -> {
                require(value.isNotEmpty() && value.toByteArray(Charsets.UTF_8).size <= MAX_HOST_SECRET_BYTES) {
                    "Provided host secret is invalid."
                }
            }
            HostSecretResolution.UNAVAILABLE,
            HostSecretResolution.SECURE_STORAGE_UNAVAILABLE,
            -> require(value.isEmpty()) { "Unavailable host secret responses must not include a secret." }
        }
        val payload = HostSecretResponse.newBuilder()
            .setRequestId(requestId)
            .setResolution(resolution.wireValue)
            .setSecret(value)
            .build()
        val envelope = Envelope.newBuilder()
            .setProtocolVersion(compatibility.protocolVersion.toInt())
            .setOperationId(operationId)
            .setCorrelationId(correlationId)
            .setSequence(0)
            .setMessageType(MESSAGE_HOST_SECRET_RESPONSE)
            .setPayload(ByteString.copyFrom(payload.toByteArray()))
            .build()
        sendHostResponse(envelope.toByteArray())
    }

    /** 请求当前操作取消且不重试。 */
    fun cancel() = withHandle { current ->
        checkResult(NativeBridge.cancel(current))
    }

    /** 停止接受新工作并取消活动操作。 */
    fun shutdown() = withHandle { current ->
        checkResult(NativeBridge.shutdown(current))
    }

    /** 关闭后阻止新调用，并等待已进入 JNI 的调用返回。 */
    override fun close() {
        lifecycleLock.write {
            val current = handle
            if (current == 0L) {
                return
            }
            handle = 0L
            val shutdownResult = NativeBridge.shutdown(current)
            val destroyResult = NativeBridge.destroy(current)
            checkResult(shutdownResult)
            checkResult(destroyResult)
        }
    }

    private inline fun <T> withHandle(block: (Long) -> T): T = lifecycleLock.read {
        val current = handle
        check(current != 0L) { "LinguaMesh engine handle is closed." }
        block(current)
    }

    companion object {
        const val ABI_VERSION_MAJOR: UInt = 1u
        const val PROTOCOL_VERSION: UInt = 1u
        const val MESSAGE_TRANSLATE_TEXT: String = "translate_text"
        const val MESSAGE_HOST_SECRET_RESPONSE: String = "host_secret_response"
        private const val MAX_HOST_REQUEST_ID_LENGTH = 128
        private const val MAX_HOST_SECRET_BYTES = 64 * 1024

        /** 验证 ABI 和协议后创建引擎。 */
        fun create(
            expectedAbiMajor: UInt = ABI_VERSION_MAJOR,
            expectedProtocolVersion: UInt = PROTOCOL_VERSION,
        ): LinguaMeshEngine {
            val actual = CoreCompatibility(
                abiMajor = NativeBridge.abiVersion().toUInt(),
                protocolVersion = NativeBridge.protocolVersion().toUInt(),
            )
            val expected = CoreCompatibility(expectedAbiMajor, expectedProtocolVersion)
            if (actual != expected) {
                throw CoreCompatibilityException(expected, actual)
            }
            val output = LongArray(1)
            checkResult(NativeBridge.create(output))
            check(output[0] != 0L) { "LinguaMesh returned an empty engine handle." }
            return LinguaMeshEngine(output[0], actual)
        }

        private fun checkResult(rawValue: Int) {
            val result = CoreResult.fromRawValue(rawValue)
            if (result != CoreResult.OK) {
                throw CoreException(result)
            }
        }
    }
}

/** 将所有未检查的 JNI 符号限制在包内。 */
internal object NativeBridge {
    init {
        System.loadLibrary("linguamesh_jni")
    }

    @JvmStatic external fun create(output: LongArray): Int
    @JvmStatic external fun abiVersion(): Int
    @JvmStatic external fun protocolVersion(): Int
    @JvmStatic external fun submit(handle: Long, command: ByteArray): Int
    @JvmStatic external fun pollEvent(handle: Long, timeoutMillis: Int, output: Array<ByteArray?>): Int
    @JvmStatic external fun sendHostResponse(handle: Long, response: ByteArray): Int
    @JvmStatic external fun cancel(handle: Long): Int
    @JvmStatic external fun shutdown(handle: Long): Int
    @JvmStatic external fun destroy(handle: Long): Int
}
