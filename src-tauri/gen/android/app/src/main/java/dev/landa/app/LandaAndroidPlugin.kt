package dev.landa.app

import android.Manifest
import android.app.Activity
import android.app.DownloadManager
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.Uri
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Environment
import android.os.ParcelFileDescriptor
import android.provider.MediaStore
import android.provider.OpenableColumns
import android.provider.Settings
import android.webkit.MimeTypeMap
import android.webkit.WebView
import androidx.appcompat.app.AppCompatActivity
import androidx.core.app.ActivityCompat
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
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
import java.net.Inet4Address
import java.net.NetworkInterface
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap

@InvokeArg
class PublishToDownloadsArgs {
    lateinit var sourcePath: String
    lateinit var relativePath: String
}

@InvokeArg
class BindSocketArgs {
    var fd: Int = -1
}

@InvokeArg
class TransferProgressNotifArgs {
    lateinit var transferId: String
    lateinit var peerName: String
    lateinit var currentFile: String
    var filesDone: Int = 0
    var fileCount: Int = 1
    var percent: Int = 0
}

@InvokeArg
class TransferCompletedNotifArgs {
    lateinit var transferId: String
    lateinit var peerName: String
    var fileCount: Int = 1
    var savedPaths: List<String> = emptyList()
}

@InvokeArg
class TransferFailedNotifArgs {
    lateinit var transferId: String
    lateinit var peerName: String
    var error: String? = null
}

@InvokeArg
class InstallApkArgs {
    lateinit var apkPath: String
}

@TauriPlugin
class LandaAndroidPlugin(private val activity: Activity) : Plugin(activity) {
    private var multicastLock: WifiManager.MulticastLock? = null
    private var webViewRef: WebView? = null
    private val pendingSharedPaths = mutableListOf<String>()
    private val lock = Any()
    private val publishedUris = ConcurrentHashMap<String, Pair<Uri, String>>()
    private val lastProgressNotifMs = ConcurrentHashMap<String, Long>()
    private var networkCallback: ConnectivityManager.NetworkCallback? = null
    private var permissionsRequestedThisSession = false

    companion object {
        private const val CHANNEL_PROGRESS = "landa_transfer_progress"
        private const val CHANNEL_DONE = "landa_transfer_done"
        private const val REQ_STARTUP_PERMS = 1002
    }

    init {
        acquireMulticastLock()
        ensureNotificationChannels()
        registerWifiNetworkCallback()
    }

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
        dispatchWebEvent("landa-network-changed")
    }

    override fun onDestroy(activity: AppCompatActivity) {
        unregisterWifiNetworkCallback()
        releaseMulticastLock()
        super.onDestroy(activity)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleShareIntent(intent)
    }

    @Command
    fun getSystemDeviceName(invoke: Invoke) {
        val name = resolveSystemDeviceName()
        val res = JSObject()
        res.put("name", name)
        invoke.resolve(res)
    }

    @Command
    fun ensureStartupPermissions(invoke: Invoke) {
        try {
            if (!permissionsRequestedThisSession) {
                permissionsRequestedThisSession = true
                val missing = mutableListOf<String>()
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    if (ContextCompat.checkSelfPermission(activity, Manifest.permission.POST_NOTIFICATIONS)
                        != PackageManager.PERMISSION_GRANTED
                    ) {
                        missing.add(Manifest.permission.POST_NOTIFICATIONS)
                    }
                    if (ContextCompat.checkSelfPermission(activity, Manifest.permission.NEARBY_WIFI_DEVICES)
                        != PackageManager.PERMISSION_GRANTED
                    ) {
                        missing.add(Manifest.permission.NEARBY_WIFI_DEVICES)
                    }
                } else {
                    if (ContextCompat.checkSelfPermission(activity, Manifest.permission.ACCESS_FINE_LOCATION)
                        != PackageManager.PERMISSION_GRANTED
                    ) {
                        missing.add(Manifest.permission.ACCESS_FINE_LOCATION)
                    }
                }
                if (Build.VERSION.SDK_INT <= Build.VERSION_CODES.P) {
                    if (ContextCompat.checkSelfPermission(activity, Manifest.permission.WRITE_EXTERNAL_STORAGE)
                        != PackageManager.PERMISSION_GRANTED
                    ) {
                        missing.add(Manifest.permission.WRITE_EXTERNAL_STORAGE)
                    }
                }
                if (missing.isNotEmpty()) {
                    ActivityCompat.requestPermissions(activity, missing.toTypedArray(), REQ_STARTUP_PERMS)
                }
            }
            val res = JSObject()
            res.put("requested", true)
            invoke.resolve(res)
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: "Failed to request permissions")
        }
    }

    @Command
    fun bindSocketToLan(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(BindSocketArgs::class.java)
            if (args.fd < 0) {
                val res = JSObject()
                res.put("bound", false)
                invoke.resolve(res)
                return
            }
            val network = findLanNetwork()
            if (network == null) {
                val res = JSObject()
                res.put("bound", false)
                invoke.resolve(res)
                return
            }
            val pfd = ParcelFileDescriptor.fromFd(args.fd)
            try {
                network.bindSocket(pfd.fileDescriptor)
            } finally {
                pfd.close()
            }
            val res = JSObject()
            res.put("bound", true)
            invoke.resolve(res)
        } catch (_: Exception) {
            val res = JSObject()
            res.put("bound", false)
            invoke.resolve(res)
        }
    }

    @Command
    fun getLanInterfaces(invoke: Invoke) {
        val arr = JSArray()
        val seenIps = HashSet<String>()

        try {
            val cm = activity.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
            if (cm != null) {
                @Suppress("DEPRECATION")
                for (net in cm.allNetworks) {
                    val caps = cm.getNetworkCapabilities(net) ?: continue
                    if (!caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)) continue
                    if (!caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) &&
                        !caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)
                    ) {
                        continue
                    }
                    val lp = cm.getLinkProperties(net) ?: continue
                    val ifaceName = lp.interfaceName ?: "wlan0"
                    for (la in lp.linkAddresses) {
                        val addr = la.address as? Inet4Address ?: continue
                        if (addr.isLoopbackAddress || addr.isLinkLocalAddress) continue
                        val ipStr = addr.hostAddress ?: continue
                        if (!seenIps.add(ipStr)) continue
                        val prefix = la.prefixLength.coerceIn(1, 32)
                        val maskInt = if (prefix == 0) 0 else (-1 shl (32 - prefix))
                        val ipInt = ipv4ToInt(addr.address)
                        val bcastInt = ipInt or maskInt.inv()
                        val obj = JSObject()
                        obj.put("name", ifaceName)
                        obj.put("ip", ipStr)
                        obj.put("netmask", intToIpv4(maskInt))
                        obj.put("broadcast", intToIpv4(bcastInt))
                        arr.put(obj)
                    }
                }
            }
        } catch (_: Exception) {
        }

        // Fallback for Wi-Fi Hotspot (ap0 / wlan1) where ConnectivityManager has no client network
        try {
            val ifaces = NetworkInterface.getNetworkInterfaces()
            while (ifaces != null && ifaces.hasMoreElements()) {
                val nif = ifaces.nextElement()
                if (!nif.isUp || nif.isLoopback) continue
                val name = nif.name ?: continue
                val lower = name.lowercase()
                if (lower.startsWith("tun") || lower.startsWith("wg") || lower.startsWith("ppp") ||
                    lower.startsWith("rmnet") || lower.startsWith("ccmni") || lower == "lo"
                ) {
                    continue
                }
                for (ia in nif.interfaceAddresses) {
                    val addr = ia.address as? Inet4Address ?: continue
                    if (addr.isLoopbackAddress || addr.isLinkLocalAddress) continue
                    val ipStr = addr.hostAddress ?: continue
                    if (!seenIps.add(ipStr)) continue
                    val prefix = ia.networkPrefixLength.toInt().coerceIn(1, 32)
                    val maskInt = -1 shl (32 - prefix)
                    val ipInt = ipv4ToInt(addr.address)
                    val bcastInt = ia.broadcast?.let { ipv4ToInt(it.address) } ?: (ipInt or maskInt.inv())
                    val obj = JSObject()
                    obj.put("name", name)
                    obj.put("ip", ipStr)
                    obj.put("netmask", intToIpv4(maskInt))
                    obj.put("broadcast", intToIpv4(bcastInt))
                    arr.put(obj)
                }
            }
        } catch (_: Exception) {
        }

        val res = JSObject()
        res.put("interfaces", arr)
        invoke.resolve(res)
    }

    @Command
    fun showTransferProgress(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(TransferProgressNotifArgs::class.java)
            if (!canPostNotifications()) {
                invoke.resolve()
                return
            }
            val now = System.currentTimeMillis()
            val prev = lastProgressNotifMs[args.transferId] ?: 0L
            if (now - prev < 350L && args.percent < 100) {
                invoke.resolve()
                return
            }
            lastProgressNotifMs[args.transferId] = now

            ensureNotificationChannels()
            val nm = activity.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            val fileName = args.currentFile.substringAfterLast('/').substringAfterLast('\\')
                .ifEmpty { args.peerName }
            val contentText = if (args.fileCount > 1) {
                "$fileName (${args.filesDone + 1}/${args.fileCount})"
            } else {
                fileName
            }

            val builder = NotificationCompat.Builder(activity, CHANNEL_PROGRESS)
                .setSmallIcon(android.R.drawable.stat_sys_download)
                .setContentTitle("Landa · ${args.peerName}")
                .setContentText(contentText)
                .setProgress(100, args.percent.coerceIn(0, 100), false)
                .setOngoing(true)
                .setOnlyAlertOnce(true)
                .setPriority(NotificationCompat.PRIORITY_LOW)

            nm.notify(notifIdFor(args.transferId), builder.build())
            invoke.resolve()
        } catch (_: Exception) {
            invoke.resolve()
        }
    }

    @Command
    fun showTransferCompleted(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(TransferCompletedNotifArgs::class.java)
            lastProgressNotifMs.remove(args.transferId)
            if (!canPostNotifications()) {
                invoke.resolve()
                return
            }

            ensureNotificationChannels()
            val nm = activity.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            val id = notifIdFor(args.transferId)
            nm.cancel(id)

            val firstPath = args.savedPaths.firstOrNull()
            val displayName = firstPath?.let {
                if (it.startsWith("content://")) {
                    queryDisplayName(Uri.parse(it)) ?: "Download"
                } else {
                    File(it).name
                }
            } ?: "Download"

            val contentText = if (args.fileCount > 1) {
                "$displayName (+${args.fileCount - 1})"
            } else {
                displayName
            }

            val tapIntent = buildOpenDownloadedIntent(args.savedPaths)
            val pendingIntent = PendingIntent.getActivity(
                activity,
                id,
                tapIntent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
            )

            val builder = NotificationCompat.Builder(activity, CHANNEL_DONE)
                .setSmallIcon(android.R.drawable.stat_sys_download_done)
                .setContentTitle("Landa · ${args.peerName}")
                .setContentText(contentText)
                .setAutoCancel(true)
                .setContentIntent(pendingIntent)
                .setPriority(NotificationCompat.PRIORITY_DEFAULT)

            nm.notify(id, builder.build())
            invoke.resolve()
        } catch (_: Exception) {
            invoke.resolve()
        }
    }

    @Command
    fun showTransferFailed(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(TransferFailedNotifArgs::class.java)
            lastProgressNotifMs.remove(args.transferId)
            if (!canPostNotifications()) {
                invoke.resolve()
                return
            }

            ensureNotificationChannels()
            val nm = activity.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            val id = notifIdFor(args.transferId)
            nm.cancel(id)

            val builder = NotificationCompat.Builder(activity, CHANNEL_DONE)
                .setSmallIcon(android.R.drawable.stat_notify_error)
                .setContentTitle("Landa · ${args.peerName}")
                .setContentText(args.error ?: "Transfer failed")
                .setAutoCancel(true)
                .setPriority(NotificationCompat.PRIORITY_DEFAULT)

            nm.notify(id, builder.build())
            invoke.resolve()
        } catch (_: Exception) {
            invoke.resolve()
        }
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
    fun installApkUpdate(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(InstallApkArgs::class.java)
            val apkFile = File(args.apkPath).canonicalFile
            val cacheRoot = activity.cacheDir.canonicalFile
            val filesRoot = activity.filesDir.canonicalFile
            val inAllowedDir = apkFile.path.startsWith(cacheRoot.path + File.separator) ||
                apkFile.path.startsWith(filesRoot.path + File.separator)
            if (!inAllowedDir || !apkFile.exists() || !apkFile.isFile) {
                invoke.reject("Invalid APK update path")
                return
            }

            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O &&
                !activity.packageManager.canRequestPackageInstalls()
            ) {
                val settingsIntent = Intent(
                    Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                    Uri.parse("package:${activity.packageName}")
                ).apply {
                    addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                }
                activity.startActivity(settingsIntent)
                invoke.reject("INSTALL_PERMISSION_REQUIRED")
                return
            }

            val uri = FileProvider.getUriForFile(
                activity,
                "${activity.packageName}.fileprovider",
                apkFile
            )
            val installIntent = Intent(Intent.ACTION_VIEW).apply {
                setDataAndType(uri, "application/vnd.android.package-archive")
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
            }
            activity.startActivity(installIntent)
            invoke.resolve()
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: "Failed to launch APK installer")
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
            val key = resolvedPath ?: uri.toString()
            publishedUris[key] = Pair(uri, mimeType)
            return key
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
        val ext = target.extension.lowercase()
        val mimeType = MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext)
            ?: "application/octet-stream"
        val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.fileprovider", target)
        publishedUris[target.absolutePath] = Pair(uri, mimeType)
        return target.absolutePath
    }

    private fun buildOpenDownloadedIntent(savedPaths: List<String>): Intent {
        if (savedPaths.size == 1) {
            val path = savedPaths[0]
            val cached = publishedUris[path]
            if (cached != null) {
                return Intent(Intent.ACTION_VIEW).apply {
                    setDataAndType(cached.first, cached.second)
                    addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
                }
            }
            if (path.startsWith("content://")) {
                val uri = Uri.parse(path)
                val mime = activity.contentResolver.getType(uri) ?: "*/*"
                return Intent(Intent.ACTION_VIEW).apply {
                    setDataAndType(uri, mime)
                    addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
                }
            }
            val file = File(path)
            if (file.exists()) {
                return try {
                    val uri = FileProvider.getUriForFile(
                        activity,
                        "${activity.packageName}.fileprovider",
                        file
                    )
                    val ext = file.extension.lowercase()
                    val mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext) ?: "*/*"
                    Intent(Intent.ACTION_VIEW).apply {
                        setDataAndType(uri, mime)
                        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
                    }
                } catch (_: Exception) {
                    Intent(DownloadManager.ACTION_VIEW_DOWNLOADS).apply {
                        addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                    }
                }
            }
        }
        return Intent(DownloadManager.ACTION_VIEW_DOWNLOADS).apply {
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        }
    }

    private fun ensureNotificationChannels() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val nm = activity.getSystemService(Context.NOTIFICATION_SERVICE) as? NotificationManager ?: return
        if (nm.getNotificationChannel(CHANNEL_PROGRESS) == null) {
            val ch = NotificationChannel(
                CHANNEL_PROGRESS,
                "Transfer Progress",
                NotificationManager.IMPORTANCE_LOW
            ).apply {
                setShowBadge(false)
            }
            nm.createNotificationChannel(ch)
        }
        if (nm.getNotificationChannel(CHANNEL_DONE) == null) {
            val ch = NotificationChannel(
                CHANNEL_DONE,
                "Completed Transfers",
                NotificationManager.IMPORTANCE_DEFAULT
            )
            nm.createNotificationChannel(ch)
        }
    }

    private fun canPostNotifications(): Boolean {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            return ContextCompat.checkSelfPermission(
                activity,
                Manifest.permission.POST_NOTIFICATIONS
            ) == PackageManager.PERMISSION_GRANTED
        }
        return true
    }

    private fun notifIdFor(transferId: String): Int {
        return 20000 + (transferId.hashCode() and 0x7FFF)
    }

    private fun resolveSystemDeviceName(): String {
        val candidates = listOfNotNull(
            runCatching {
                Settings.Global.getString(activity.contentResolver, "device_name")
            }.getOrNull(),
            runCatching {
                Settings.Secure.getString(activity.contentResolver, "bluetooth_name")
            }.getOrNull(),
            runCatching {
                val mfg = Build.MANUFACTURER?.trim().orEmpty()
                val model = Build.MODEL?.trim().orEmpty()
                when {
                    model.isEmpty() -> null
                    mfg.isEmpty() || model.startsWith(mfg, ignoreCase = true) -> model
                    else -> "$mfg $model"
                }
            }.getOrNull()
        )
        return candidates.firstOrNull {
            it.isNotBlank() && !it.equals("localhost", ignoreCase = true)
        } ?: "Android"
    }

    private fun findLanNetwork(): Network? {
        val cm = activity.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
            ?: return null
        @Suppress("DEPRECATION")
        val networks = cm.allNetworks
        var fallbackEthernet: Network? = null
        for (net in networks) {
            val caps = cm.getNetworkCapabilities(net) ?: continue
            if (!caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)) continue
            if (caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)) {
                return net
            }
            if (caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)) {
                fallbackEthernet = net
            }
        }
        return fallbackEthernet
    }

    private fun registerWifiNetworkCallback() {
        try {
            val cm = activity.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
                ?: return
            val req = NetworkRequest.Builder()
                .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
                .addCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
                .build()
            val cb = object : ConnectivityManager.NetworkCallback() {
                override fun onAvailable(network: Network) {
                    dispatchWebEvent("landa-network-changed")
                }

                override fun onLost(network: Network) {
                    dispatchWebEvent("landa-network-changed")
                }
            }
            cm.registerNetworkCallback(req, cb)
            networkCallback = cb
        } catch (_: Exception) {
        }
    }

    private fun unregisterWifiNetworkCallback() {
        try {
            val cm = activity.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
                ?: return
            networkCallback?.let { cm.unregisterNetworkCallback(it) }
        } catch (_: Exception) {
        } finally {
            networkCallback = null
        }
    }

    private fun dispatchWebEvent(eventName: String) {
        activity.runOnUiThread {
            webViewRef?.evaluateJavascript(
                "window.dispatchEvent(new Event('$eventName'))",
                null
            )
        }
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
            dispatchWebEvent("landa-share-intent")
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

    private fun ipv4ToInt(bytes: ByteArray): Int {
        return ((bytes[0].toInt() and 0xFF) shl 24) or
            ((bytes[1].toInt() and 0xFF) shl 16) or
            ((bytes[2].toInt() and 0xFF) shl 8) or
            (bytes[3].toInt() and 0xFF)
    }

    private fun intToIpv4(value: Int): String {
        return "${(value ushr 24) and 0xFF}.${(value ushr 16) and 0xFF}.${(value ushr 8) and 0xFF}.${value and 0xFF}"
    }
}
