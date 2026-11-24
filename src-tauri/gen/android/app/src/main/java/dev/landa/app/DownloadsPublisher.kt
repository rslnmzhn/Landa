package dev.landa.app

import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream

internal data class PublishedDownload(
    val publishedPath: String,
    val displayPath: String,
)

internal object DownloadsPublisher {
    fun publishLegacyFile(targetBase: File, sourceFile: File, cleanRel: String): File {
        var target = File(targetBase, cleanRel)
        target.parentFile?.mkdirs()
        if (target.exists()) {
            val parent = target.parentFile ?: targetBase
            val name = target.name
            val dot = name.lastIndexOf('.')
            val stem = if (dot > 0) name.substring(0, dot) else name
            val ext = if (dot > 0) name.substring(dot) else ""
            var idx = 1
            while (target.exists()) {
                target = File(parent, "$stem ($idx)$ext")
                idx++
            }
        }
        FileInputStream(sourceFile).use { input ->
            FileOutputStream(target).use { out ->
                input.copyTo(out, 64 * 1024)
            }
        }
        return target
    }

    fun formatLegacyDisplayPath(targetBase: File, target: File): String {
        val rel = target.relativeTo(targetBase).invariantSeparatorsPath
        return "Downloads/Landa/$rel"
    }

    fun formatMediaStoreDisplayPath(
        parts: List<String>,
        actualDisplayName: String?,
        resolvedPath: String?,
        fallbackFileName: String,
    ): String {
        val finalName = actualDisplayName?.trim()?.ifEmpty { null }
            ?: resolvedPath?.let { File(it).name }?.trim()?.ifEmpty { null }
            ?: fallbackFileName
        val displayRel = if (parts.size > 1) {
            (parts.dropLast(1) + finalName).joinToString("/")
        } else {
            finalName
        }
        return "Downloads/Landa/$displayRel"
    }

    fun formatResolvedMediaStorePath(
        relativePath: String?,
        displayName: String?,
    ): String? {
        val cleanName = displayName?.trim()?.ifEmpty { null } ?: return null
        val rawRel = relativePath?.replace('\\', '/')?.trim()?.trim('/')?.ifEmpty { null }
            ?: return null
        val normalizedRel = when {
            rawRel == "Download" -> "Downloads"
            rawRel.startsWith("Download/") -> "Downloads/" + rawRel.removePrefix("Download/")
            else -> rawRel
        }
        return "$normalizedRel/$cleanName"
    }

    fun selectMediaStorePublishedPath(
        contentUri: String,
        @Suppress("UNUSED_PARAMETER") resolvedPath: String?,
    ): String {
        return contentUri
    }

    fun parseDownloadRelativeQuery(path: String): Pair<String, String>? {
        val normalized = path.replace('\\', '/').trim()
        val marker = "/Download/"
        val idx = normalized.indexOf(marker)
        if (idx < 0) return null
        val afterDownload = normalized.substring(idx + marker.length).trimStart('/')
        if (afterDownload.isEmpty()) return null
        val lastSlash = afterDownload.lastIndexOf('/')
        return if (lastSlash < 0) {
            Pair("Download/", afterDownload)
        } else {
            val subDir = afterDownload.substring(0, lastSlash)
            val fileName = afterDownload.substring(lastSlash + 1)
            if (fileName.isEmpty()) null else Pair("Download/$subDir/", fileName)
        }
    }
}
