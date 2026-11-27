package dev.landa.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.File

class SafShareHelperTest {
    @Test
    fun resolvesNonEmptyStreamWhenColumnSizeNullAndStatSizeUnavailable() {
        val payload = "hello saf".toByteArray()
        val size = SafShareHelper.resolveSafFileSize(
            fileName = "note.txt",
            cursorSize = null,
            statSizeProvider = { -1L },
            streamByteCounter = { SafShareHelper.countStreamBytes(ByteArrayInputStream(payload)) }
        )
        assertEquals(payload.size.toLong(), size)
    }

    @Test
    fun resolvesNonEmptyStreamWhenColumnSizeZeroAndStatSizeUnavailable() {
        val payload = ByteArray(128) { 7 }
        val size = SafShareHelper.resolveSafFileSize(
            fileName = "photo.jpg",
            cursorSize = 0L,
            statSizeProvider = { -1L },
            streamByteCounter = { SafShareHelper.countStreamBytes(ByteArrayInputStream(payload)) }
        )
        assertEquals(128L, size)
    }

    @Test
    fun confirmsTrulyEmptyFileAsZero() {
        val size = SafShareHelper.resolveSafFileSize(
            fileName = "empty.txt",
            cursorSize = 0L,
            statSizeProvider = { 0L },
            streamByteCounter = { SafShareHelper.countStreamBytes(ByteArrayInputStream(ByteArray(0))) }
        )
        assertEquals(0L, size)
    }

    @Test
    fun throwsWithFileNameWhenSizeCannotBeDetermined() {
        val ex = assertThrows(IllegalStateException::class.java) {
            SafShareHelper.resolveSafFileSize(
                fileName = "broken:file.bin",
                cursorSize = null,
                statSizeProvider = { -1L },
                streamByteCounter = { null }
            )
        }
        assertTrue(ex.message.orEmpty().contains("broken:file.bin"))
    }

    @Test
    fun preservesTrailingSpacesToDistinguishDocuments() {
        val plain = SafShareHelper.normalizeSafDisplayName("a")
        val withSpace = SafShareHelper.normalizeSafDisplayName("a ")
        assertEquals("a", plain)
        assertEquals("a ", withSpace)
        assertNotEquals(plain, withSpace)

        assertEquals(listOf("dir", "a "), SafShareHelper.parseSafeRelSegments("dir/a "))
        assertEquals(listOf("dir", "a"), SafShareHelper.parseSafeRelSegments("dir/a"))
    }

    @Test
    fun copiesOpenedStreamToTempFileWithExactByteCount() {
        val tempFile = File.createTempFile("saf_helper_test_", ".bin")
        try {
            val payload = "twelve-bytes".toByteArray()
            val copied = SafShareHelper.copyStreamToTempFile(ByteArrayInputStream(payload), tempFile)
            assertEquals(12L, copied)
            assertEquals("twelve-bytes", tempFile.readText())

            val emptyCopied = SafShareHelper.copyStreamToTempFile(ByteArrayInputStream(ByteArray(0)), tempFile)
            assertEquals(0L, emptyCopied)
            assertEquals(0L, tempFile.length())
        } finally {
            tempFile.delete()
        }
    }
}
