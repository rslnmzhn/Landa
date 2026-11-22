package dev.landa.app

import java.io.File
import java.nio.file.Files
import org.junit.Assert.assertEquals
import org.junit.Test

class DownloadsPublisherTest {
    @Test
    fun legacyPublishDeduplicatesExistingApkAndReturnsNewDisplayPath() {
        val tempRoot = Files.createTempDirectory("landa-downloads-test").toFile()
        try {
            val downloadsLanda = File(tempRoot, "Downloads/Landa").apply { mkdirs() }
            val existingApk = File(downloadsLanda, "landa-v0.5.10-android-arm64-v8a.apk")
            existingApk.writeText("old-apk-bytes")

            val sourceApk = File(tempRoot, "verified-update.apk")
            sourceApk.writeText("new-apk-bytes")

            val published = DownloadsPublisher.publishLegacyFile(
                targetBase = downloadsLanda,
                sourceFile = sourceApk,
                cleanRel = "landa-v0.5.10-android-arm64-v8a.apk",
            )
            val displayPath = DownloadsPublisher.formatLegacyDisplayPath(downloadsLanda, published)

            assertEquals("landa-v0.5.10-android-arm64-v8a (1).apk", published.name)
            assertEquals("Downloads/Landa/landa-v0.5.10-android-arm64-v8a (1).apk", displayPath)
            assertEquals("old-apk-bytes", existingApk.readText())
            assertEquals("new-apk-bytes", published.readText())

            val thirdSource = File(tempRoot, "verified-update-3.apk").apply {
                writeText("third-apk-bytes")
            }
            val thirdPublished = DownloadsPublisher.publishLegacyFile(
                targetBase = downloadsLanda,
                sourceFile = thirdSource,
                cleanRel = "landa-v0.5.10-android-arm64-v8a.apk",
            )
            val thirdDisplayPath = DownloadsPublisher.formatLegacyDisplayPath(downloadsLanda, thirdPublished)
            assertEquals("Downloads/Landa/landa-v0.5.10-android-arm64-v8a (2).apk", thirdDisplayPath)
        } finally {
            tempRoot.deleteRecursively()
        }
    }

    @Test
    fun mediaStoreDisplayPathUsesActualDisplayNameWhenDeduplicated() {
        val displayPath = DownloadsPublisher.formatMediaStoreDisplayPath(
            parts = listOf("landa-v0.5.10-android-arm64-v8a.apk"),
            actualDisplayName = "landa-v0.5.10-android-arm64-v8a (1).apk",
            resolvedPath = "/storage/emulated/0/Download/Landa/landa-v0.5.10-android-arm64-v8a (1).apk",
            fallbackFileName = "landa-v0.5.10-android-arm64-v8a.apk",
        )
        assertEquals("Downloads/Landa/landa-v0.5.10-android-arm64-v8a (1).apk", displayPath)

        val fallbackFromData = DownloadsPublisher.formatMediaStoreDisplayPath(
            parts = listOf("landa-v0.5.10-android-arm64-v8a.apk"),
            actualDisplayName = null,
            resolvedPath = "/storage/emulated/0/Download/Landa/landa-v0.5.10-android-arm64-v8a (2).apk",
            fallbackFileName = "landa-v0.5.10-android-arm64-v8a.apk",
        )
        assertEquals("Downloads/Landa/landa-v0.5.10-android-arm64-v8a (2).apk", fallbackFromData)
    }
}
