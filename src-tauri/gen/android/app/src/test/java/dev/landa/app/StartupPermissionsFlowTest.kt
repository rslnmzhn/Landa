package dev.landa.app

import org.junit.Assert.assertEquals
import org.junit.Test

class StartupPermissionsFlowTest {
    @Test
    fun rejectsWithoutTriggeringScanWhenPluginHandleIsNullAndPermissionsMissing() {
        val coordinator = StartupPermissionsCoordinator<String>()
        val events = mutableListOf<String>()

        coordinator.onRequest(
            token = "invoke-1",
            missingPermissions = listOf("android.permission.NEARBY_WIFI_DEVICES"),
            hasPluginHandle = false,
            requestWithHandle = { events.add("request") },
            onComplete = { tokens -> events.add("complete:${tokens.joinToString(",")}") },
            onReject = { token, msg -> events.add("reject:$token:$msg") },
        )

        assertEquals(StartupPermissionsState.NOT_REQUESTED, coordinator.state)
        assertEquals(
            listOf("reject:invoke-1:Plugin handle unavailable for permission request"),
            events,
        )
    }

    @Test
    fun secondCallWhileInProgressDoesNotOpenSecondDialogOrCompleteBeforeCallback() {
        val coordinator = StartupPermissionsCoordinator<String>()
        val events = mutableListOf<String>()
        val missing = listOf(
            "android.permission.POST_NOTIFICATIONS",
            "android.permission.NEARBY_WIFI_DEVICES",
        )

        coordinator.onRequest(
            token = "invoke-1",
            missingPermissions = missing,
            hasPluginHandle = true,
            requestWithHandle = { perms -> events.add("request:${perms.joinToString(",")}") },
            onComplete = { tokens -> events.add("complete:${tokens.joinToString(",")}") },
            onReject = { token, msg -> events.add("reject:$token:$msg") },
        )
        assertEquals(StartupPermissionsState.IN_PROGRESS, coordinator.state)

        // Second call arrives before user responds to the permission dialog
        coordinator.onRequest(
            token = "invoke-2",
            missingPermissions = missing,
            hasPluginHandle = true,
            requestWithHandle = { perms -> events.add("request:${perms.joinToString(",")}") },
            onComplete = { tokens -> events.add("complete:${tokens.joinToString(",")}") },
            onReject = { token, msg -> events.add("reject:$token:$msg") },
        )

        // Still in progress: only 1 dialog opened, neither invoke completed yet
        assertEquals(StartupPermissionsState.IN_PROGRESS, coordinator.state)
        assertEquals(
            listOf("request:android.permission.POST_NOTIFICATIONS,android.permission.NEARBY_WIFI_DEVICES"),
            events,
        )

        // Single permission callback arrives -> both pending invokes complete together
        coordinator.onPermissionCallback("invoke-1") { tokens ->
            events.add("complete:${tokens.joinToString(",")}")
        }
        assertEquals(StartupPermissionsState.COMPLETED, coordinator.state)
        assertEquals(
            listOf(
                "request:android.permission.POST_NOTIFICATIONS,android.permission.NEARBY_WIFI_DEVICES",
                "complete:invoke-1,invoke-2",
            ),
            events,
        )

        // Third call after completion completes immediately without opening a dialog
        coordinator.onRequest(
            token = "invoke-3",
            missingPermissions = missing,
            hasPluginHandle = true,
            requestWithHandle = { events.add("request-unexpected") },
            onComplete = { tokens -> events.add("complete:${tokens.joinToString(",")}") },
            onReject = { token, msg -> events.add("reject:$token:$msg") },
        )
        assertEquals(
            listOf(
                "request:android.permission.POST_NOTIFICATIONS,android.permission.NEARBY_WIFI_DEVICES",
                "complete:invoke-1,invoke-2",
                "complete:invoke-3",
            ),
            events,
        )
    }

    @Test
    fun singlePermissionCallbackWithRegisteredListenerTriggersExactlyOneScanForTwoWaitingRequests() {
        val coordinator = StartupPermissionsCoordinator<String>()
        val missing = listOf("android.permission.NEARBY_WIFI_DEVICES")
        var totalScanCount = 0
        var pendingNetworkChanged = false
        var lockAcquireCount = 0

        val completeWithListener: (List<String>) -> Unit = { invokes ->
            val resolutions = StartupPermissionsCompletion.complete(
                invokes = invokes,
                isDestroyed = false,
                hasListener = true,
                onAcquireLocks = { lockAcquireCount++ },
                onTriggerNetworkChanged = { source ->
                    NetworkChangedDispatcher.dispatch(
                        source = source,
                        hasListener = true,
                        setPending = { pendingNetworkChanged = it },
                        notifyNativeListener = { totalScanCount++ },
                        notifyWebView = { totalScanCount++ },
                    )
                },
            )
            for (res in resolutions) {
                if (!res.listenerNotified) {
                    totalScanCount++
                }
            }
        }

        coordinator.onRequest(
            token = "invoke-1",
            missingPermissions = missing,
            hasPluginHandle = true,
            requestWithHandle = {},
            onComplete = completeWithListener,
            onReject = { _, _ -> },
        )
        coordinator.onRequest(
            token = "invoke-2",
            missingPermissions = missing,
            hasPluginHandle = true,
            requestWithHandle = {},
            onComplete = completeWithListener,
            onReject = { _, _ -> },
        )
        assertEquals(0, totalScanCount)

        coordinator.onPermissionCallback("invoke-1", completeWithListener)

        assertEquals(1, lockAcquireCount)
        assertEquals(false, pendingNetworkChanged)
        assertEquals(1, totalScanCount)
    }

    @Test
    fun startupPermissionsWithoutListenerFollowedByListenerRegistrationKeepsTotalScanCountAtOne() {
        val coordinator = StartupPermissionsCoordinator<String>()
        val missing = listOf("android.permission.NEARBY_WIFI_DEVICES")
        val webViewReady = true
        var hasListener = false
        var pendingNetworkChanged = false
        var totalScanCount = 0
        var rustFallbackClaimed = false

        val completeWithoutListener: (List<String>) -> Unit = { invokes ->
            val resolutions = StartupPermissionsCompletion.complete(
                invokes = invokes,
                isDestroyed = false,
                hasListener = hasListener,
                onAcquireLocks = {},
                onTriggerNetworkChanged = { source ->
                    NetworkChangedDispatcher.dispatch(
                        source = source,
                        hasListener = hasListener,
                        setPending = { pendingNetworkChanged = it },
                        notifyNativeListener = { totalScanCount++ },
                        notifyWebView = {
                            if (webViewReady) {
                                totalScanCount++
                            }
                        },
                    )
                },
            )
            for (res in resolutions) {
                if (!res.listenerNotified && !rustFallbackClaimed) {
                    rustFallbackClaimed = true
                    totalScanCount++
                }
            }
        }

        coordinator.onRequest(
            token = "invoke-1",
            missingPermissions = missing,
            hasPluginHandle = true,
            requestWithHandle = {},
            onComplete = completeWithoutListener,
            onReject = { _, _ -> },
        )
        coordinator.onRequest(
            token = "invoke-2",
            missingPermissions = missing,
            hasPluginHandle = true,
            requestWithHandle = {},
            onComplete = completeWithoutListener,
            onReject = { _, _ -> },
        )

        coordinator.onPermissionCallback("invoke-1", completeWithoutListener)
        assertEquals(false, pendingNetworkChanged)
        assertEquals(1, totalScanCount)

        // Later, registerNetworkListener succeeds and flushes pendingNetworkChanged if set.
        hasListener = true
        if (pendingNetworkChanged && hasListener) {
            pendingNetworkChanged = false
            totalScanCount++
        }

        assertEquals(false, pendingNetworkChanged)
        assertEquals(1, totalScanCount)
    }

    @Test
    fun loadAndResumeStillDispatchWebViewFallbackWhenNativeListenerMissing() {
        for (source in listOf(
            NetworkChangeSource.LOAD,
            NetworkChangeSource.RESUME,
            NetworkChangeSource.NETWORK_CALLBACK,
        )) {
            var pendingNetworkChanged = false
            var webViewScanCount = 0
            var nativeScanCount = 0

            NetworkChangedDispatcher.dispatch(
                source = source,
                hasListener = false,
                setPending = { pendingNetworkChanged = it },
                notifyNativeListener = { nativeScanCount++ },
                notifyWebView = { webViewScanCount++ },
            )

            assertEquals(true, pendingNetworkChanged)
            assertEquals(0, nativeScanCount)
            assertEquals(1, webViewScanCount)
        }
    }

    @Test
    fun triggersReadyImmediatelyWhenNoPermissionsMissing() {
        val coordinator = StartupPermissionsCoordinator<String>()
        val events = mutableListOf<String>()

        coordinator.onRequest(
            token = "invoke-1",
            missingPermissions = emptyList(),
            hasPluginHandle = false,
            requestWithHandle = { events.add("request") },
            onComplete = { tokens -> events.add("complete:${tokens.joinToString(",")}") },
            onReject = { token, msg -> events.add("reject:$token:$msg") },
        )

        assertEquals(StartupPermissionsState.COMPLETED, coordinator.state)
        assertEquals(listOf("complete:invoke-1"), events)
    }
}
