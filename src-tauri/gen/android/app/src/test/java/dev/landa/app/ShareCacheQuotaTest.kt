package dev.landa.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.File
import java.nio.file.Files

class ShareCacheQuotaTest {
    @Test
    fun singleFileLargerThanLimitIsRejectedAndDeleted() {
        val dir = Files.createTempDirectory("landa-share-quota-single").toFile()
        try {
            val maxBytes = 100L * 1024L
            val oversized = ByteArray(120 * 1024) { 0x41 }

            // Unknown size (knownSizeBytes = null): must abort mid-stream and delete partial file.
            val result = ShareCacheQuota.copyStreamWithQuota(
                input = ByteArrayInputStream(oversized),
                shareDir = dir,
                targetName = "oversized.bin",
                maxBytes = maxBytes,
                knownSizeBytes = null
            )

            assertNull("oversized file must be rejected", result)
            assertEquals(
                "no partial file may remain in share_intent",
                0,
                dir.listFiles()?.size ?: 0
            )
        } finally {
            dir.deleteRecursively()
        }
    }

    @Test
    fun batchExceedingLimitEvictsOldestAndOnlyPublishesExistingFiles() {
        val dir = Files.createTempDirectory("landa-share-quota-batch").toFile()
        try {
            val maxBytes = 100L * 1024L
            val chunk40k = ByteArray(40 * 1024) { 0x42 }
            val copied = mutableListOf<String>()

            val f1 = ShareCacheQuota.copyStreamWithQuota(
                ByteArrayInputStream(chunk40k),
                dir,
                "file1.bin",
                maxBytes
            )!!
            f1.setLastModified(1_000L)
            copied.add(f1.absolutePath)

            val f2 = ShareCacheQuota.copyStreamWithQuota(
                ByteArrayInputStream(chunk40k),
                dir,
                "file2.bin",
                maxBytes
            )!!
            f2.setLastModified(2_000L)
            copied.add(f2.absolutePath)

            val f3 = ShareCacheQuota.copyStreamWithQuota(
                ByteArrayInputStream(chunk40k),
                dir,
                "file3.bin",
                maxBytes
            )!!
            f3.setLastModified(3_000L)
            copied.add(f3.absolutePath)

            val published = ShareCacheQuota.filterExistingPaths(copied)
            val totalOnDisk = dir.listFiles()?.filter { it.isFile }?.sumOf { it.length() } ?: 0L

            assertTrue("total size on disk must stay within quota", totalOnDisk <= maxBytes)
            assertEquals(listOf(f2.absolutePath, f3.absolutePath), published)
            assertTrue("every published path must exist on disk", published.all { File(it).exists() })
        } finally {
            dir.deleteRecursively()
        }
    }
}
