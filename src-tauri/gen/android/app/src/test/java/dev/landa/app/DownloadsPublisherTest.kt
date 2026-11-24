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

    @Test
    fun mediaStorePublishedPathPrefersContentUriOverDataPath() {
        val published = DownloadsPublisher.selectMediaStorePublishedPath(
            contentUri = "content://media/external/downloads/1000012345",
            resolvedPath = "/storage/emulated/0/Download/Landa/photo.jpg",
        )
        assertEquals("content://media/external/downloads/1000012345", published)
    }

    @Test
    fun parsesDownloadRelativeQueryFromLegacyMediaStorePath() {
        assertEquals(
            Pair("Download/Landa/", "photo.jpg"),
            DownloadsPublisher.parseDownloadRelativeQuery("/storage/emulated/0/Download/Landa/photo.jpg"),
        )
        assertEquals(
            Pair("Download/Landa/Vacation/", "clip.mp4"),
            DownloadsPublisher.parseDownloadRelativeQuery("/storage/emulated/0/Download/Landa/Vacation/clip.mp4"),
        )
        assertEquals(
            null,
            DownloadsPublisher.parseDownloadRelativeQuery("/data/user/0/dev.landa.app/files/note.txt"),
        )
    }

    @Test
    fun formatsResolvedMediaStoreRelativePathAndDisplayName() {
        assertEquals(
            "Downloads/Landa/photo.jpg",
            DownloadsPublisher.formatResolvedMediaStorePath("Download/Landa/", "photo.jpg"),
        )
        assertEquals(
            "Downloads/Landa/Trip/day1/beach.png",
            DownloadsPublisher.formatResolvedMediaStorePath("Download/Landa/Trip/day1/", "beach.png"),
        )
        assertEquals(
            "Downloads/photo.jpg",
            DownloadsPublisher.formatResolvedMediaStorePath("Download", "photo.jpg"),
        )
        assertEquals(
            null,
            DownloadsPublisher.formatResolvedMediaStorePath(null, "photo.jpg"),
        )
        assertEquals(
            null,
            DownloadsPublisher.formatResolvedMediaStorePath("   ", "photo.jpg"),
        )
        assertEquals(
            null,
            DownloadsPublisher.formatResolvedMediaStorePath("Download/Landa/", null),
        )
        assertEquals(
            null,
            DownloadsPublisher.formatResolvedMediaStorePath("Download/Landa/", "   "),
        )
    }
}
