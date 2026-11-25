package dev.landa.app

internal enum class StartupPermissionsState {
    NOT_REQUESTED,
    IN_PROGRESS,
    COMPLETED,
}

internal data class StartupPermissionResolution<T>(
    val token: T,
    val listenerNotified: Boolean,
)

internal enum class NetworkChangeSource {
    STARTUP_PERMISSIONS,
    LOAD,
    RESUME,
    NETWORK_CALLBACK,
}

internal object NetworkChangedDispatcher {
    fun dispatch(
        source: NetworkChangeSource,
        hasListener: Boolean,
        setPending: (Boolean) -> Unit,
        notifyNativeListener: () -> Unit,
        notifyWebView: () -> Unit,
    ) {
        if (hasListener) {
            setPending(false)
            notifyNativeListener()
            return
        }
        when (source) {
            NetworkChangeSource.STARTUP_PERMISSIONS -> {
                // Rust ensure_startup_permissions owns the single fallback scan;
                // leave pendingNetworkChanged untouched so later listener registration
                // does not trigger a duplicate scan for startup permissions.
            }
            NetworkChangeSource.LOAD,
            NetworkChangeSource.RESUME,
            NetworkChangeSource.NETWORK_CALLBACK -> {
                setPending(true)
                notifyWebView()
            }
        }
    }
}

internal object StartupPermissionsCompletion {
    fun <T> complete(
        invokes: List<T>,
        isDestroyed: Boolean,
        hasListener: Boolean,
        onAcquireLocks: () -> Unit,
        onTriggerNetworkChanged: (NetworkChangeSource) -> Unit,
    ): List<StartupPermissionResolution<T>> {
        val listenerNotified = if (!isDestroyed) {
            onAcquireLocks()
            onTriggerNetworkChanged(NetworkChangeSource.STARTUP_PERMISSIONS)
            hasListener
        } else {
            false
        }
        return invokes.mapIndexed { idx, token ->
            StartupPermissionResolution(
                token = token,
                listenerNotified = listenerNotified || idx > 0,
            )
        }
    }
}

internal class StartupPermissionsCoordinator<T> {
    var state: StartupPermissionsState = StartupPermissionsState.NOT_REQUESTED
        private set
    private val pending = mutableListOf<T>()

    fun onRequest(
        token: T,
        missingPermissions: List<String>,
        hasPluginHandle: Boolean,
        requestWithHandle: (Array<String>) -> Unit,
        onComplete: (List<T>) -> Unit,
        onReject: (T, String) -> Unit,
    ) {
        when (state) {
            StartupPermissionsState.COMPLETED -> onComplete(listOf(token))
            StartupPermissionsState.IN_PROGRESS -> {
                if (pending.none { it === token }) {
                    pending.add(token)
                }
            }
            StartupPermissionsState.NOT_REQUESTED -> {
                if (missingPermissions.isEmpty()) {
                    state = StartupPermissionsState.COMPLETED
                    onComplete(listOf(token))
                    return
                }
                if (!hasPluginHandle) {
                    onReject(token, "Plugin handle unavailable for permission request")
                    return
                }
                state = StartupPermissionsState.IN_PROGRESS
                pending.add(token)
                try {
                    requestWithHandle(missingPermissions.toTypedArray())
                } catch (ex: Exception) {
                    state = StartupPermissionsState.NOT_REQUESTED
                    val failed = pending.toList()
                    pending.clear()
                    val msg = ex.message ?: "Failed to request permissions"
                    for (item in failed) {
                        onReject(item, msg)
                    }
                }
            }
        }
    }

    fun onPermissionCallback(
        callbackToken: T,
        onComplete: (List<T>) -> Unit,
    ) {
        state = StartupPermissionsState.COMPLETED
        val toResolve = ArrayList<T>(pending.size + 1)
        toResolve.addAll(pending)
        if (toResolve.none { it === callbackToken }) {
            toResolve.add(callbackToken)
        }
        pending.clear()
        onComplete(toResolve)
    }

    fun clearPending(onReject: (T) -> Unit) {
        val toReject = pending.toList()
        pending.clear()
        if (state == StartupPermissionsState.IN_PROGRESS) {
            state = StartupPermissionsState.NOT_REQUESTED
        }
        for (item in toReject) {
            onReject(item)
        }
    }
}
