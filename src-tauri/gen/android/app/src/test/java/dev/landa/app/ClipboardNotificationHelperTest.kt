package dev.landa.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ClipboardNotificationHelperTest {
    @Test
    fun returnsFailureAndDoesNotResolveSuccessWhenNotifyThrows() {
        val outcomes = mutableListOf<String>()

        ClipboardNotificationPoster.post(
            appInForeground = false,
            canPostNotifications = true,
            postNotification = {
                throw SecurityException("Notification posting blocked by OS")
            },
            onSuccess = { shown -> outcomes.add("success:$shown") },
            onFailure = { err -> outcomes.add("failure:$err") },
        )

        assertEquals(1, outcomes.size)
        assertTrue(outcomes[0].startsWith("failure:Failed to show clipboard notification:"))
        assertTrue(outcomes[0].contains("Notification posting blocked by OS"))
    }

    @Test
    fun returnsShownTrueWhenNotifySucceeds() {
        val outcomes = mutableListOf<String>()

        ClipboardNotificationPoster.post(
            appInForeground = false,
            canPostNotifications = true,
            postNotification = {},
            onSuccess = { shown -> outcomes.add("success:$shown") },
            onFailure = { err -> outcomes.add("failure:$err") },
        )

        assertEquals(listOf("success:true"), outcomes)
    }

    @Test
    fun returnsShownFalseWithoutPostingWhenForegroundOrPermissionMissing() {
        val outcomes = mutableListOf<String>()
        var postCalls = 0

        ClipboardNotificationPoster.post(
            appInForeground = true,
            canPostNotifications = true,
            postNotification = { postCalls++ },
            onSuccess = { shown -> outcomes.add("fg:$shown") },
            onFailure = { err -> outcomes.add("failure:$err") },
        )
        ClipboardNotificationPoster.post(
            appInForeground = false,
            canPostNotifications = false,
            postNotification = { postCalls++ },
            onSuccess = { shown -> outcomes.add("perm:$shown") },
            onFailure = { err -> outcomes.add("failure:$err") },
        )

        assertEquals(0, postCalls)
        assertEquals(listOf("fg:false", "perm:false"), outcomes)
    }

    @Test
    fun selectsPngAfterPdfInMixedClipData() {
        val idx = ClipboardClipSelector.selectImageItemIndex(
            items = listOf(
                true to "application/pdf",
                true to "image/png",
            ),
            clipHasImageMime = true,
        )
        assertEquals(1, idx)
    }

    @Test
    fun doesNotSelectKnownNonImageUriEvenWhenClipHasImageMime() {
        val idx = ClipboardClipSelector.selectImageItemIndex(
            items = listOf(
                true to "application/pdf",
                false to null,
            ),
            clipHasImageMime = true,
        )
        assertEquals(null, idx)
    }

    @Test
    fun fallsBackToUnknownMimeUriOnlyWhenClipHasImageMime() {
        val idx = ClipboardClipSelector.selectImageItemIndex(
            items = listOf(
                true to "application/pdf",
                true to null,
            ),
            clipHasImageMime = true,
        )
        assertEquals(1, idx)
    }
}
