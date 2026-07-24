package org.linguamesh.core

import com.google.protobuf.ByteString
import org.linguamesh.core.protocol.Envelope
import org.linguamesh.core.protocol.FailureEvent
import org.linguamesh.core.protocol.SecretRequiredEvent
import org.linguamesh.core.protocol.TextDeltaEvent
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

class CoreEventTest {
    @Test
    fun decodesTextDeltaWithoutExposingProtocolType() {
        val payload = TextDeltaEvent.newBuilder().setText("你好").build()
        val event = decodeCoreEvent(envelope("text_delta", payload.toByteString()))

        assertTrue(event is CoreEvent.TextDelta)
        val delta = event as CoreEvent.TextDelta
        assertEquals(CoreEventKind.TEXT_DELTA, delta.kind)
        assertEquals("你好", delta.text)
        assertEquals(4uL, delta.sequence)
    }

    @Test
    fun decodesStructuredFailure() {
        val payload = FailureEvent.newBuilder()
            .setErrorKind("provider")
            .setMessage("Provider rejected the request.")
            .build()
        val event = decodeCoreEvent(envelope("failed", payload.toByteString()))

        assertTrue(event is CoreEvent.Failed)
        val failed = event as CoreEvent.Failed
        assertEquals(CoreEventKind.FAILED, failed.kind)
        assertEquals("provider", failed.errorKind)
        assertEquals("Provider rejected the request.", failed.message)
    }

    @Test
    fun decodesTypedSecretRequestWithoutExposingProtocolType() {
        val payload = SecretRequiredEvent.newBuilder()
            .setRequestId("request-1")
            .setSecretRef("provider/profile-1")
            .build()
        val event = decodeCoreEvent(envelope("secret_required", payload.toByteString()))

        assertTrue(event is CoreEvent.SecretRequired)
        val required = event as CoreEvent.SecretRequired
        assertEquals(CoreEventKind.SECRET_REQUIRED, required.kind)
        assertEquals("request-1", required.requestId)
        assertEquals("provider/profile-1", required.secretRef)
    }

    @Test
    fun preservesUnknownEventForForwardCompatibility() {
        val payload = byteArrayOf(1, 2, 3)
        val event = decodeCoreEvent(envelope("future_event", ByteString.copyFrom(payload)))

        assertTrue(event is CoreEvent.Unknown)
        val unknown = event as CoreEvent.Unknown
        assertEquals(CoreEventKind.UNKNOWN, unknown.kind)
        assertEquals("future_event", unknown.messageType)
        assertArrayEquals(payload, unknown.payload)
    }

    @Test
    fun rejectsMalformedTypedPayload() {
        val error = assertThrows(CoreException::class.java) {
            decodeCoreEvent(envelope("text_delta", ByteString.copyFrom(byteArrayOf(0x0a, 0x02))))
        }

        assertEquals(CoreResult.MALFORMED_MESSAGE, error.result)
    }

    private fun envelope(messageType: String, payload: ByteString): Envelope = Envelope.newBuilder()
        .setProtocolVersion(1)
        .setOperationId("operation-1")
        .setCorrelationId("correlation-1")
        .setSequence(4)
        .setMessageType(messageType)
        .setPayload(payload)
        .build()
}
