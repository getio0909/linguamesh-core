package org.linguamesh.core

import com.google.protobuf.InvalidProtocolBufferException
import org.linguamesh.core.protocol.Envelope
import org.linguamesh.core.protocol.FailureEvent
import org.linguamesh.core.protocol.TextDeltaEvent

/** 表示无需了解 Protobuf 细节即可处理的稳定事件类别。 */
enum class CoreEventKind {
    STARTED,
    TEXT_DELTA,
    COMPLETED,
    CANCELLED,
    FAILED,
    UNKNOWN,
}

/** 表示核心发出的已解码事件。 */
sealed class CoreEvent(
    open val operationId: String,
    open val correlationId: String,
    open val sequence: ULong,
    val kind: CoreEventKind,
) {
    /** 表示操作已开始。 */
    data class Started(
        override val operationId: String,
        override val correlationId: String,
        override val sequence: ULong,
    ) : CoreEvent(operationId, correlationId, sequence, CoreEventKind.STARTED)

    /** 表示新到达的一段增量文本。 */
    data class TextDelta(
        override val operationId: String,
        override val correlationId: String,
        override val sequence: ULong,
        val text: String,
    ) : CoreEvent(operationId, correlationId, sequence, CoreEventKind.TEXT_DELTA)

    /** 表示操作已成功完成。 */
    data class Completed(
        override val operationId: String,
        override val correlationId: String,
        override val sequence: ULong,
    ) : CoreEvent(operationId, correlationId, sequence, CoreEventKind.COMPLETED)

    /** 表示操作已在保留现有增量后取消。 */
    data class Cancelled(
        override val operationId: String,
        override val correlationId: String,
        override val sequence: ULong,
    ) : CoreEvent(operationId, correlationId, sequence, CoreEventKind.CANCELLED)

    /** 表示操作以结构化错误终止。 */
    data class Failed(
        override val operationId: String,
        override val correlationId: String,
        override val sequence: ULong,
        val errorKind: String,
        val message: String,
    ) : CoreEvent(operationId, correlationId, sequence, CoreEventKind.FAILED)

    /** 保留来自较新协议的未知事件以便诊断或转发。 */
    data class Unknown(
        override val operationId: String,
        override val correlationId: String,
        override val sequence: ULong,
        val messageType: String,
        val payload: ByteArray,
    ) : CoreEvent(operationId, correlationId, sequence, CoreEventKind.UNKNOWN)
}

/** 将已验证的封套转换为不暴露生成类型的公共事件。 */
internal fun decodeCoreEvent(envelope: Envelope): CoreEvent {
    val operationId = envelope.operationId
    val correlationId = envelope.correlationId
    val sequence = envelope.sequence.toULong()
    return try {
        when (envelope.messageType) {
            MESSAGE_STARTED -> CoreEvent.Started(operationId, correlationId, sequence)
            MESSAGE_TEXT_DELTA -> {
                val event = TextDeltaEvent.parseFrom(envelope.payload)
                CoreEvent.TextDelta(operationId, correlationId, sequence, event.text)
            }
            MESSAGE_COMPLETED -> CoreEvent.Completed(operationId, correlationId, sequence)
            MESSAGE_CANCELLED -> CoreEvent.Cancelled(operationId, correlationId, sequence)
            MESSAGE_FAILED -> {
                val event = FailureEvent.parseFrom(envelope.payload)
                CoreEvent.Failed(
                    operationId,
                    correlationId,
                    sequence,
                    event.errorKind,
                    event.message,
                )
            }
            else -> CoreEvent.Unknown(
                operationId,
                correlationId,
                sequence,
                envelope.messageType,
                envelope.payload.toByteArray(),
            )
        }
    } catch (cause: InvalidProtocolBufferException) {
        throw CoreException(CoreResult.MALFORMED_MESSAGE, cause)
    }
}

private const val MESSAGE_STARTED = "started"
private const val MESSAGE_TEXT_DELTA = "text_delta"
private const val MESSAGE_COMPLETED = "completed"
private const val MESSAGE_CANCELLED = "cancelled"
private const val MESSAGE_FAILED = "failed"
