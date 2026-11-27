package dev.landa.app

internal object ClipboardNotificationPoster {
    fun post(
        appInForeground: Boolean,
        canPostNotifications: Boolean,
        postNotification: () -> Unit,
        onSuccess: (shown: Boolean) -> Unit,
        onFailure: (error: String) -> Unit,
    ) {
        if (appInForeground || !canPostNotifications) {
            onSuccess(false)
            return
        }
        try {
            postNotification()
            onSuccess(true)
        } catch (ex: Exception) {
            val detail = ex.message?.takeIf { it.isNotBlank() } ?: ex.javaClass.simpleName
            onFailure("Failed to show clipboard notification: $detail")
        }
    }
}

internal object ClipboardClipSelector {
    fun selectImageItemIndex(
        items: List<Pair<Boolean, String?>>,
        clipHasImageMime: Boolean,
    ): Int? {
        for ((index, entry) in items.withIndex()) {
            val (hasUri, mime) = entry
            if (hasUri && mime?.startsWith("image/", ignoreCase = true) == true) {
                return index
            }
        }
        if (clipHasImageMime) {
            for ((index, entry) in items.withIndex()) {
                val (hasUri, mime) = entry
                if (hasUri && mime == null) {
                    return index
                }
            }
        }
        return null
    }
}
