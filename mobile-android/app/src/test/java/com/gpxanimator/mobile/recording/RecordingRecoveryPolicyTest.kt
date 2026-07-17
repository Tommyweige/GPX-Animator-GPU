package com.gpxanimator.mobile.recording

import org.junit.Assert.assertEquals
import org.junit.Test

class RecordingRecoveryPolicyTest {
    @Test
    fun `running service always keeps the ride active`() {
        assertEquals(
            RecordingRecoveryAction.KEEP_RUNNING,
            RecordingRecoveryPolicy.decide(
                serviceRunning = true,
                sameBoot = false,
                leaseFresh = false,
            ),
        )
    }

    @Test
    fun `fresh lease on the same boot recovers the service`() {
        assertEquals(
            RecordingRecoveryAction.RECOVER_SERVICE,
            RecordingRecoveryPolicy.decide(
                serviceRunning = false,
                sameBoot = true,
                leaseFresh = true,
            ),
        )
    }

    @Test
    fun `stale lease or boot change marks the ride interrupted`() {
        assertEquals(
            RecordingRecoveryAction.MARK_INTERRUPTED,
            RecordingRecoveryPolicy.decide(
                serviceRunning = false,
                sameBoot = true,
                leaseFresh = false,
            ),
        )
        assertEquals(
            RecordingRecoveryAction.MARK_INTERRUPTED,
            RecordingRecoveryPolicy.decide(
                serviceRunning = false,
                sameBoot = false,
                leaseFresh = true,
            ),
        )
    }
}
