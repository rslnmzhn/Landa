package dev.landa.app

import java.io.File
import java.io.FileOutputStream
import java.io.InputStream

internal object ShareCacheQuota {
    const val MAX_SHARE_INTENT_CACHE_BYTES: Long = 100L * 1024L * 1024L
    private const val CHUNK_SIZE = 64 * 1024

    fun copyStreamWithQuota(
        input: InputStream,
        shareDir: File,
        targetName: String,
        maxBytes: Long = MAX_SHARE_INTENT_CACHE_BYTES,
        knownSizeBytes: Long? = null
    ): File? {
        if (knownSizeBytes != null && knownSizeBytes > maxBytes) {
            return null
        }
        shareDir.mkdirs()
        val target = File(shareDir, targetName)
        val buffer = ByteArray(CHUNK_SIZE)
        var completed = false
        try {
            FileOutputStream(target).use { out ->
                while (true) {
                    val read = input.read(buffer)
                    if (read <= 0) break
                    if (!ensureSpaceForChunk(shareDir, target, read.toLong(), maxBytes)) {
                        return null
                    }
                    out.write(buffer, 0, read)
                }
                out.flush()
            }
            completed = true
            return target
        } catch (_: Exception) {
            return null
        } finally {
            if (!completed) {
                target.delete()
            }
        }
    }

    fun filterExistingPaths(paths: List<String>): List<String> {
        return paths.filter { path ->
            val f = File(path)
            f.exists() && f.isFile
        }
    }

    fun enforceLimit(shareDir: File, maxBytes: Long = MAX_SHARE_INTENT_CACHE_BYTES) {
        val files = shareDir.listFiles()?.filter { it.isFile } ?: return
        var totalBytes = files.sumOf { it.length() }
        if (totalBytes <= maxBytes) return

        val sortedOldestFirst = files.sortedBy { it.lastModified() }
        for (file in sortedOldestFirst) {
            if (totalBytes <= maxBytes) break
            val len = file.length()
            if (file.delete()) {
                totalBytes -= len
            }
        }
    }

    private fun ensureSpaceForChunk(
        shareDir: File,
        activeTarget: File,
        chunkBytes: Long,
        maxBytes: Long
    ): Boolean {
        val files = shareDir.listFiles()?.filter { it.isFile } ?: emptyList()
        var totalBytes = files.sumOf { it.length() }
        if (totalBytes + chunkBytes <= maxBytes) {
            return true
        }

        val evictable = files
            .filter { it.absolutePath != activeTarget.absolutePath }
            .sortedBy { it.lastModified() }

        for (candidate in evictable) {
            if (totalBytes + chunkBytes <= maxBytes) {
                return true
            }
            val len = candidate.length()
            if (candidate.delete()) {
                totalBytes -= len
            }
        }

        return totalBytes + chunkBytes <= maxBytes
    }
}
