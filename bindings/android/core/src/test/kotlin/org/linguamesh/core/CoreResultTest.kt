package org.linguamesh.core

import org.junit.Assert.assertEquals
import org.junit.Test

class CoreResultTest {
    @Test
    fun mapsStableAndUnknownResultCodes() {
        assertEquals(CoreResult.OK, CoreResult.fromRawValue(0))
        assertEquals(CoreResult.BUSY, CoreResult.fromRawValue(6))
        assertEquals(CoreResult.RESOURCE_EXHAUSTED, CoreResult.fromRawValue(9))
        assertEquals(CoreResult.UNKNOWN, CoreResult.fromRawValue(99))
    }
}
