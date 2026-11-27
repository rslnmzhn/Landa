package dev.landa.app

import java.io.File
import java.io.FileOutputStream
import java.io.InputStream

internal object SafShareHelper {
    fun normalizeSafDisplayName(rawName: String?): String? {
        val name = rawName ?: return null
        if (name.isEmpty() || name == "." || name == ".." ||
            name.contains('/') || name.contains('\\') || name.contains('\u0000')
        ) {
            return null
        }
        return name
    }

    fun parseSafeRelSegments(relPath: String): List<String> {
        if (relPath.isEmpty() || relPath == "." || relPath == "/") {
            return emptyList()
        }
        if (relPath.contains('\\') || relPath.contains('\u0000')) {
            throw IllegalArgumentException("Invalid relative path")
        }
        val parts = relPath.split('/').filter { it.isNotEmpty() }
        for (part in parts) {
            if (part == "." || part == "..") {
                throw IllegalArgumentException("Path traversal is not allowed")
            }
        }
        return parts
    }

    fun resolveSafFileSize(
        fileName: String,
        cursorSize: Long?,
        statSizeProvider: () -> Long,
        streamByteCounter: () -> Long?
    ): Long {
        if (cursorSize != null && cursorSize > 0L) {
            return cursorSize
        }
        val statSize = statSizeProvider()
        if (statSize > 0L) {
            return statSize
        }
        val counted = streamByteCounter()
        if (counted != null && counted >= 0L) {
            return counted
        }
        throw IllegalStateException("Unable to determine size for shared file: $fileName")
    }

    fun countStreamBytes(stream: InputStream): Long {
        val buf = ByteArray(64 * 1024)
        var total = 0L
        while (true) {
            val n = stream.read(buf)
            if (n < 0) break
            total += n
        }
        return total
    }

    fun copyStreamToTempFile(stream: InputStream, tempFile: File): Long {
        val buf = ByteArray(64 * 1024)
        var total = 0L
        FileOutputStream(tempFile).use { out ->
            while (true) {
                val n = stream.read(buf)
                if (n < 0) break
                out.write(buf, 0, n)
                total += n
            }
            out.flush()
        }
        return total
    }
}
