package dev.landa.app

import android.Manifest
import android.app.Activity
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.provider.OpenableColumns
import android.provider.Settings
import android.webkit.MimeTypeMap
import android.webkit.WebView
import androidx.appcompat.app.AppCompatActivity
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream
import java.util.UUID

@InvokeArg
class PublishToDownloadsArgs {
    lateinit var sourcePath: String
    lateinit var relativePath: String
}

@TauriPlugin
class LandaAndroidPlugin(private val activity: Activity) : Plugin(activity) {
    private var multicastLock: WifiManager.MulticastLock? = null
    private var webViewRef: WebView? = null
    private val pendingSharedPaths = mutableListOf<String>()
    private val lock = Any()

    override fun load(webView: WebView) {
        super.load(webView)
        webViewRef = webView
        acquireMulticastLock()
        sweepOldShareCache()
        activity.intent?.let { handleShareIntent(it) }
    }

    override fun onResume(activity: AppCompatActivity) {
        super.onResume(activity)
        acquireMulticastLock()
    }

    override fun onDestroy(activity: AppCompatActivity) {
        releaseMulticastLock()
        super.onDestroy(activity)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleShareIntent(intent)
    }

    @Command
    fun takePendingSharedFiles(invoke: Invoke) {
        val paths: List<String>
        synchronized(lock) {
            paths = pendingSharedPaths.toList()
            pendingSharedPaths.clear()
        }
        val arr = JSArray()
        for (p in paths) {
            arr.put(p)
        }
        val res = JSObject()
        res.put("paths", arr)
        invoke.resolve(res)
    }

    @Command
    fun hasAllFilesAccess(invoke: Invoke) {
        val granted = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            Environment.isExternalStorageManager()
        } else {
            ContextCompat.checkSelfPermission(
                activity,
                Manifest.permission.READ_EXTERNAL_STORAGE
            ) == PackageManager.PERMISSION_GRANTED
        }
        val res = JSObject()
        res.put("granted", granted)
        invoke.resolve(res)
    }

    @Command
    fun requestAllFilesAccess(invoke: Invoke) {
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                if (!Environment.isExternalStorageManager()) {
                    val intent = try {
                        Intent(
                            Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION,
                            Uri.parse("package:${activity.packageName}")
                        )
                    } catch (_: Exception) {
                        Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION)
                    }
                    activity.startActivity(intent)
                }
            } else {
                ActivityCompat.requestPermissions(
                    activity,
                    arrayOf(
                        Manifest.permission.READ_EXTERNAL_STORAGE,
                        Manifest.permission.WRITE_EXTERNAL_STORAGE
                    ),
                    1001
                )
            }
            invoke.resolve()
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: "Failed to request storage permission")
        }
    }

    @Command
    fun publishToDownloads(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(PublishToDownloadsArgs::class.java)
            val sourceFile = File(args.sourcePath)
            if (!sourceFile.exists() || !sourceFile.isFile) {
                invoke.reject("Source file does not exist: ${args.sourcePath}")
                return
            }

            val cleanRel = sanitizeRelativePath(args.relativePath)
            val publishedPath = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                publishViaMediaStore(sourceFile, cleanRel)
            } else {
                publishLegacyDownloads(sourceFile, cleanRel)
            }

            // Remove private sandbox copy once published to public Downloads
            sourceFile.delete()

            val res = JSObject()
            res.put("publishedPath", publishedPath)
            invoke.resolve(res)
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: "Failed to publish file to Downloads")
        }
    }

    private fun publishViaMediaStore(sourceFile: File, cleanRel: String): String {
        val parts = cleanRel.split('/').filter { it.isNotEmpty() }
        val fileName = parts.lastOrNull() ?: sourceFile.name
        val subDir = if (parts.size > 1) {
            "Landa/" + parts.dropLast(1).joinToString("/")
        } else {
            "Landa"
        }
        val relativeDir = "${Environment.DIRECTORY_DOWNLOADS}/$subDir"
        val ext = fileName.substringAfterLast('.', "").lowercase()
        val mimeType = MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext)
            ?: "application/octet-stream"

        val values = ContentValues().apply {
            put(MediaStore.Downloads.DISPLAY_NAME, fileName)
            put(MediaStore.Downloads.MIME_TYPE, mimeType)
            put(MediaStore.Downloads.RELATIVE_PATH, relativeDir)
            put(MediaStore.Downloads.IS_PENDING, 1)
        }

        val resolver = activity.contentResolver
        val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
            ?: throw IllegalStateException("MediaStore insert returned null")

        try {
            resolver.openOutputStream(uri)?.use { out ->
                FileInputStream(sourceFile).use { input ->
                    input.copyTo(out, 64 * 1024)
                }
            } ?: throw IllegalStateException("Failed to open MediaStore output stream")

            val doneValues = ContentValues().apply {
                put(MediaStore.Downloads.IS_PENDING, 0)
            }
            resolver.update(uri, doneValues, null, null)

            var resolvedPath: String? = null
            resolver.query(
                uri,
                arrayOf(MediaStore.MediaColumns.DATA),
                null,
                null,
                null
            )?.use { cursor ->
                if (cursor.moveToFirst()) {
                    val idx = cursor.getColumnIndex(MediaStore.MediaColumns.DATA)
                    if (idx >= 0) {
                        resolvedPath = cursor.getString(idx)
                    }
                }
            }
            return resolvedPath ?: uri.toString()
        } catch (ex: Exception) {
            resolver.delete(uri, null, null)
            throw ex
        }
    }

    @Suppress("DEPRECATION")
    private fun publishLegacyDownloads(sourceFile: File, cleanRel: String): String {
        val downloadsRoot = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS)
        val targetBase = File(downloadsRoot, "Landa")
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
        return target.absolutePath
    }

    private fun acquireMulticastLock() {
        try {
            if (multicastLock?.isHeld == true) return
            val wifi = activity.applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager
                ?: return
            val lock = wifi.createMulticastLock("landa_multicast_lock").apply {
                setReferenceCounted(false)
                acquire()
            }
            multicastLock = lock
        } catch (_: Exception) {
            // Ignore if Wi-Fi service is unavailable
        }
    }

    private fun releaseMulticastLock() {
        try {
            if (multicastLock?.isHeld == true) {
                multicastLock?.release()
            }
        } catch (_: Exception) {
        } finally {
            multicastLock = null
        }
    }

    private fun handleShareIntent(intent: Intent) {
        val action = intent.action ?: return
        if (action != Intent.ACTION_SEND && action != Intent.ACTION_SEND_MULTIPLE) {
            return
        }

        val uris = mutableListOf<Uri>()
        if (action == Intent.ACTION_SEND) {
            @Suppress("DEPRECATION")
            val single = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
            } else {
                intent.getParcelableExtra(Intent.EXTRA_STREAM)
            }
            if (single != null) uris.add(single)
        } else {
            @Suppress("DEPRECATION")
            val list = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java)
            } else {
                intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM)
            }
            if (list != null) uris.addAll(list)
        }

        if (uris.isEmpty()) {
            intent.clipData?.let { clip ->
                for (i in 0 until clip.itemCount) {
                    clip.getItemAt(i).uri?.let { uris.add(it) }
                }
            }
        }

        if (uris.isEmpty()) return

        val shareDir = File(activity.cacheDir, "share_intent").apply { mkdirs() }
        val copied = mutableListOf<String>()
        for (uri in uris) {
            copyUriToCache(uri, shareDir)?.let { copied.add(it) }
        }

        intent.action = null
        if (copied.isNotEmpty()) {
            synchronized(lock) {
                pendingSharedPaths.addAll(copied)
            }
            val arr = JSArray()
            for (p in copied) arr.put(p)
            val payload = JSObject()
            payload.put("paths", arr)
            trigger("shareIntent", payload)
            activity.runOnUiThread {
                webViewRef?.evaluateJavascript(
                    "window.dispatchEvent(new Event('landa-share-intent'))",
                    null
                )
            }
        }
    }

    private fun copyUriToCache(uri: Uri, shareDir: File): String? {
        return try {
            val rawName = queryDisplayName(uri) ?: "shared_${UUID.randomUUID().toString().take(8)}"
            val safeName = rawName.replace(Regex("[^A-Za-z0-9._-]"), "_").take(100)
                .ifEmpty { "shared_file" }
            val target = File(shareDir, "${UUID.randomUUID().toString().take(8)}_$safeName")
            activity.contentResolver.openInputStream(uri)?.use { input ->
                FileOutputStream(target).use { out ->
                    input.copyTo(out, 64 * 1024)
                }
            } ?: return null
            target.absolutePath
        } catch (_: Exception) {
            null
        }
    }

    private fun queryDisplayName(uri: Uri): String? {
        return try {
            activity.contentResolver.query(
                uri,
                arrayOf(OpenableColumns.DISPLAY_NAME),
                null,
                null,
                null
            )?.use { cursor ->
                if (cursor.moveToFirst()) {
                    val idx = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                    if (idx >= 0) cursor.getString(idx) else null
                } else null
            }
        } catch (_: Exception) {
            null
        }
    }

    private fun sweepOldShareCache() {
        try {
            val shareDir = File(activity.cacheDir, "share_intent")
            if (!shareDir.exists()) return
            val cutoff = System.currentTimeMillis() - 24L * 60L * 60L * 1000L
            shareDir.listFiles()?.forEach { file ->
                if (file.isFile && file.lastModified() < cutoff) {
                    file.delete()
                }
            }
        } catch (_: Exception) {
        }
    }

    private fun sanitizeRelativePath(raw: String): String {
        return raw.replace('\\', '/')
            .split('/')
            .map { it.trim() }
            .filter { it.isNotEmpty() && it != "." && it != ".." }
            .joinToString("/")
            .ifEmpty { "file.bin" }
    }
}
