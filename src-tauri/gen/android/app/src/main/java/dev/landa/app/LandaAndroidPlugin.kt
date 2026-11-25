package dev.landa.app

import android.Manifest
import android.app.Activity
import android.app.DownloadManager
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.ClipData
import android.content.ClipboardManager
import android.content.ContentUris
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
import android.os.Handler
import android.os.Looper
import android.os.ParcelFileDescriptor
import android.provider.DocumentsContract
import android.provider.MediaStore
import android.provider.OpenableColumns
import android.provider.Settings
import android.webkit.MimeTypeMap
import android.webkit.WebView
import androidx.activity.result.ActivityResult
import androidx.appcompat.app.AppCompatActivity
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.PermissionCallback
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

@InvokeArg
class ListSafShareDirArgs {
    lateinit var rootUri: String
    var isDir: Boolean = true
    var relPath: String = ""
}

@InvokeArg
class ListSafShareTreeArgs {
    lateinit var rootUri: String
    var isDir: Boolean = true
    var relPath: String = ""
    var maxDepth: Int = 32
    var maxFiles: Int = 10_000
}

@InvokeArg
class OpenSafShareFileArgs {
    lateinit var rootUri: String
    var isDir: Boolean = true
    var relPath: String = ""
}

@InvokeArg
class OpenContentUriArgs {
    lateinit var uri: String
}

@InvokeArg
class ResolveContentUriDisplayNamesArgs {
    var uris: List<String> = emptyList()
}

@InvokeArg
class TransferForegroundArgs {
    var activeCount: Int = 0
    lateinit var direction: String
    lateinit var peerName: String
    lateinit var currentFile: String
    var filesDone: Int = 0
    var fileCount: Int = 1
    var percent: Int = 0
}

@InvokeArg
class TransferOfferNotifArgs {
    lateinit var transferId: String
    lateinit var peerName: String
    var fileCount: Int = 1
    var totalBytes: Long = 0L
    var fileNames: List<String> = emptyList()
}

@InvokeArg
class CancelTransferOfferNotifArgs {
    lateinit var transferId: String
}

@InvokeArg
class FriendRequestNotifArgs {
    lateinit var peerId: String
    lateinit var deviceName: String
    lateinit var sasCode: String
}

@InvokeArg
class CancelFriendRequestNotifArgs {
    lateinit var peerId: String
}

@InvokeArg
class WriteClipboardTextArgs {
    lateinit var text: String
}

@InvokeArg
class WriteClipboardImageArgs {
    lateinit var imagePath: String
}

@TauriPlugin
class LandaAndroidPlugin(private val activity: Activity) : Plugin(activity) {
    private var multicastLock: WifiManager.MulticastLock? = null
    private var wifiLock: WifiManager.WifiLock? = null
    private val mainHandler = Handler(Looper.getMainLooper())
    private var webViewRef: WebView? = null
    private val pendingSharedPaths = mutableListOf<String>()
    private var pendingFocusTarget: JSObject? = null
    private val lock = Any()
    private val publishedUris = ConcurrentHashMap<String, Pair<Uri, String>>()
    private val publishedDisplayPaths = ConcurrentHashMap<String, String>()
    private val lastProgressNotifMs = ConcurrentHashMap<String, Long>()
    private var lastFgUpdateMs: Long = 0L
    private var lastFgActiveCount: Int = 0
    private var fgServiceRunning: Boolean = false
    @Volatile
    private var appInForeground: Boolean = true
    @Volatile
    private var isDestroyed: Boolean = false
    @Volatile
    private var pendingNetworkChanged: Boolean = false
    private var networkCallback: ConnectivityManager.NetworkCallback? = null
    private val startupPermissionsCoordinator = StartupPermissionsCoordinator<Invoke>()

    companion object {
        private const val CHANNEL_PROGRESS = "landa_transfer_progress"
        private const val CHANNEL_DONE = "landa_transfer_done"
        private const val CHANNEL_REQUESTS = "landa_incoming_requests"
        private const val MAX_FOLDER_FILES = 10_000
        private const val MAX_FOLDER_DEPTH = 32
    }

    init {
        runOnMainThread {
            if (isDestroyed) return@runOnMainThread
            acquireMulticastLock()
            acquireWifiLock()
        }
        ensureNotificationChannels()
        registerWifiNetworkCallback()
    }

    override fun load(webView: WebView) {
        super.load(webView)
        webViewRef = webView
        runOnMainThread {
            if (isDestroyed) return@runOnMainThread
            appInForeground = true
            acquireMulticastLock(forceReacquire = true)
            acquireWifiLock()
            triggerNetworkChanged(NetworkChangeSource.LOAD)
        }
        sweepOldShareCache()
        activity.intent?.let {
            handleShareIntent(it)
            handleFocusIntent(it)
        }
    }

    override fun onResume(activity: AppCompatActivity) {
        super.onResume(activity)
        runOnMainThread {
            if (isDestroyed) return@runOnMainThread
            appInForeground = true
            acquireMulticastLock(forceReacquire = true)
            acquireWifiLock()
            triggerNetworkChanged(NetworkChangeSource.RESUME)
        }
    }

    override fun onPause(activity: AppCompatActivity) {
        runOnMainThread {
            appInForeground = false
            releaseWifiLock()
        }
        super.onPause(activity)
    }

    override fun onDestroy(activity: AppCompatActivity) {
        runOnMainThread {
            isDestroyed = true
            appInForeground = false
            startupPermissionsCoordinator.clearPending { pendingInvoke ->
                pendingInvoke.reject("Plugin destroyed")
            }
            unregisterWifiNetworkCallback()
            releaseWifiLock()
            releaseMulticastLock()
            mainHandler.removeCallbacksAndMessages(null)
        }
        stopTransferForegroundInternal()
        super.onDestroy(activity)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleShareIntent(intent)
        handleFocusIntent(intent)
    }

    @Command
    fun getSystemDeviceName(invoke: Invoke) {
        val name = resolveSystemDeviceName()
        val res = JSObject()
        res.put("name", name)
        invoke.resolve(res)
    }

    @Command
    fun registerNetworkListener(invoke: Invoke) {
        super.registerListener(invoke)
        mainHandler.post {
            if (!isDestroyed && pendingNetworkChanged && hasListener("network-changed")) {
                pendingNetworkChanged = false
                trigger("network-changed", JSObject())
            }
        }
    }

    @Command
    fun ensureStartupPermissions(invoke: Invoke) {
        runOnMainThread {
            if (isDestroyed) {
                invoke.reject("Plugin destroyed")
                return@runOnMainThread
            }
            try {
                val missing = mutableListOf<String>()
                if (startupPermissionsCoordinator.state == StartupPermissionsState.NOT_REQUESTED) {
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
                }

                val pluginHandle = handle
                startupPermissionsCoordinator.onRequest(
                    token = invoke,
                    missingPermissions = missing,
                    hasPluginHandle = pluginHandle != null,
                    requestWithHandle = { perms ->
                        pluginHandle!!.requestPermissions(
                            invoke,
                            perms,
                            "onStartupPermissionsResult"
                        )
                    },
                    onComplete = { invokes ->
                        completeStartupPermissions(invokes)
                    },
                    onReject = { targetInvoke, reason ->
                        targetInvoke.reject(reason)
                    },
                )
            } catch (ex: Exception) {
                invoke.reject(ex.message ?: "Failed to request permissions")
            }
        }
    }

    @PermissionCallback
    fun onStartupPermissionsResult(invoke: Invoke) {
        runOnMainThread {
            startupPermissionsCoordinator.onPermissionCallback(invoke) { invokes ->
                completeStartupPermissions(invokes)
            }
        }
    }

    private fun completeStartupPermissions(invokes: List<Invoke>) {
        val resolutions = StartupPermissionsCompletion.complete(
            invokes = invokes,
            isDestroyed = isDestroyed,
            hasListener = hasListener("network-changed"),
            onAcquireLocks = {
                acquireMulticastLock(forceReacquire = true)
                if (appInForeground) {
                    acquireWifiLock()
                }
            },
            onTriggerNetworkChanged = { source ->
                triggerNetworkChanged(source)
            },
        )
        for (resolution in resolutions) {
            val res = JSObject()
            res.put("requested", true)
            res.put("listenerNotified", resolution.listenerNotified)
            resolution.token.resolve(res)
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
    fun isAppInForeground(invoke: Invoke) {
        val res = JSObject()
        res.put("inForeground", appInForeground)
        invoke.resolve(res)
    }

    @Command
    fun updateTransferForeground(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(TransferForegroundArgs::class.java)
            if (args.activeCount <= 0) {
                stopTransferForegroundInternal()
                invoke.resolve()
                return
            }

            val now = System.currentTimeMillis()
            if (fgServiceRunning &&
                lastFgActiveCount == args.activeCount &&
                now - lastFgUpdateMs < 350L &&
                args.percent < 100
            ) {
                invoke.resolve()
                return
            }
            lastFgUpdateMs = now
            lastFgActiveCount = args.activeCount

            val serviceIntent = Intent(activity, TransferForegroundService::class.java).apply {
                action = TransferForegroundService.ACTION_START_OR_UPDATE
                putExtra(TransferForegroundService.EXTRA_ACTIVE_COUNT, args.activeCount)
                putExtra(TransferForegroundService.EXTRA_DIRECTION, args.direction)
                putExtra(TransferForegroundService.EXTRA_PEER_NAME, args.peerName)
                putExtra(TransferForegroundService.EXTRA_CURRENT_FILE, args.currentFile)
                putExtra(TransferForegroundService.EXTRA_FILES_DONE, args.filesDone)
                putExtra(TransferForegroundService.EXTRA_FILE_COUNT, args.fileCount)
                putExtra(TransferForegroundService.EXTRA_PERCENT, args.percent)
            }

            try {
                ContextCompat.startForegroundService(activity, serviceIntent)
                fgServiceRunning = true
            } catch (_: Exception) {
                // Android 12+ may restrict starting a foreground service from background;
                // fall back to a regular progress notification without crashing.
                if (canPostNotifications()) {
                    val nm = activity.getSystemService(Context.NOTIFICATION_SERVICE) as? NotificationManager
                    val notif = TransferForegroundService.buildTransferNotification(
                        context = activity,
                        activeCount = args.activeCount,
                        direction = args.direction,
                        peerName = args.peerName,
                        currentFile = args.currentFile,
                        filesDone = args.filesDone,
                        fileCount = args.fileCount,
                        percent = args.percent
                    )
                    nm?.notify(TransferForegroundService.FG_NOTIFICATION_ID, notif)
                }
            }
            invoke.resolve()
        } catch (_: Exception) {
            invoke.resolve()
        }
    }

    @Command
    fun stopTransferForeground(invoke: Invoke) {
        stopTransferForegroundInternal()
        invoke.resolve()
    }

    @Command
    fun showTransferOfferNotification(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(TransferOfferNotifArgs::class.java)
            if (appInForeground || !canPostNotifications()) {
                invoke.resolve()
                return
            }

            ensureNotificationChannels()
            val nm = activity.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            val id = offerNotifId(args.transferId)

            val tapIntent = Intent(activity, MainActivity::class.java).apply {
                flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
                putExtra("landa_focus_target", "transfer_offer")
                putExtra("landa_transfer_id", args.transferId)
            }
            val pendingIntent = PendingIntent.getActivity(
                activity,
                id,
                tapIntent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
            )

            val firstFile = args.fileNames.firstOrNull()
                ?.substringAfterLast('/')
                ?.substringAfterLast('\\')
            val sizeLabel = formatBytesCompact(args.totalBytes)
            val contentText = when {
                args.fileCount > 1 -> "${args.fileCount} files · $sizeLabel"
                !firstFile.isNullOrEmpty() -> "$firstFile · $sizeLabel"
                else -> sizeLabel
            }

            val builder = NotificationCompat.Builder(activity, CHANNEL_REQUESTS)
                .setSmallIcon(android.R.drawable.stat_sys_download)
                .setContentTitle("Landa · ${args.peerName}")
                .setContentText(contentText)
                .setAutoCancel(true)
                .setContentIntent(pendingIntent)
                .setPriority(NotificationCompat.PRIORITY_HIGH)
                .setCategory(NotificationCompat.CATEGORY_MESSAGE)

            nm.notify(id, builder.build())
            invoke.resolve()
        } catch (_: Exception) {
            invoke.resolve()
        }
    }

    @Command
    fun cancelTransferOfferNotification(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(CancelTransferOfferNotifArgs::class.java)
            val nm = activity.getSystemService(Context.NOTIFICATION_SERVICE) as? NotificationManager
            nm?.cancel(offerNotifId(args.transferId))
        } catch (_: Exception) {
        }
        invoke.resolve()
    }

    @Command
    fun showFriendRequestNotification(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(FriendRequestNotifArgs::class.java)
            if (appInForeground || !canPostNotifications()) {
                invoke.resolve()
                return
            }

            ensureNotificationChannels()
            val nm = activity.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            val id = friendNotifId(args.peerId)

            val tapIntent = Intent(activity, MainActivity::class.java).apply {
                flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
                putExtra("landa_focus_target", "friend_requests")
                putExtra("landa_peer_id", args.peerId)
            }
            val pendingIntent = PendingIntent.getActivity(
                activity,
                id,
                tapIntent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
            )

            val builder = NotificationCompat.Builder(activity, CHANNEL_REQUESTS)
                .setSmallIcon(android.R.drawable.stat_notify_chat)
                .setContentTitle("Landa · ${args.deviceName}")
                .setContentText("Friend request · Code ${args.sasCode}")
                .setAutoCancel(true)
                .setContentIntent(pendingIntent)
                .setPriority(NotificationCompat.PRIORITY_HIGH)
                .setCategory(NotificationCompat.CATEGORY_SOCIAL)

            nm.notify(id, builder.build())
            invoke.resolve()
        } catch (_: Exception) {
            invoke.resolve()
        }
    }

    @Command
    fun cancelFriendRequestNotification(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(CancelFriendRequestNotifArgs::class.java)
            val nm = activity.getSystemService(Context.NOTIFICATION_SERVICE) as? NotificationManager
            nm?.cancel(friendNotifId(args.peerId))
        } catch (_: Exception) {
        }
        invoke.resolve()
    }

    @Command
    fun takePendingFocusTarget(invoke: Invoke) {
        val focus: JSObject?
        synchronized(lock) {
            focus = pendingFocusTarget
            pendingFocusTarget = null
        }
        val res = JSObject()
        res.put("focus", focus)
        invoke.resolve(res)
    }

    @Command
    fun scanQrCode(invoke: Invoke) {
        try {
            val intent = Intent(activity, QrScannerActivity::class.java)
            startActivityForResult(invoke, intent, "scanQrCodeResult")
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: "Failed to launch QR scanner")
        }
    }

    @ActivityCallback
    fun scanQrCodeResult(invoke: Invoke, result: ActivityResult) {
        if (result.resultCode == Activity.RESULT_CANCELED) {
            val err = result.data?.getStringExtra(QrScannerActivity.EXTRA_ERROR)
            if (!err.isNullOrBlank()) {
                invoke.reject(err)
                return
            }
            val res = JSObject()
            res.put("cancelled", true)
            res.put("uri", null)
            invoke.resolve(res)
            return
        }
        if (result.resultCode != Activity.RESULT_OK) {
            invoke.reject("QR scan failed")
            return
        }
        val uri = result.data?.getStringExtra(QrScannerActivity.EXTRA_QR_URI)
        if (uri.isNullOrBlank()) {
            val res = JSObject()
            res.put("cancelled", true)
            res.put("uri", null)
            invoke.resolve(res)
            return
        }
        val res = JSObject()
        res.put("cancelled", false)
        res.put("uri", uri)
        invoke.resolve(res)
    }

    @Command
    fun readSystemClipboard(invoke: Invoke) {
        activity.runOnUiThread {
            try {
                val cm = activity.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager
                val clip = cm?.primaryClip
                if (clip == null || clip.itemCount == 0) {
                    val res = JSObject()
                    res.put("kind", "empty")
                    invoke.resolve(res)
                    return@runOnUiThread
                }

                val item = clip.getItemAt(0)
                val uri = item.uri
                val mime = uri?.let { activity.contentResolver.getType(it) }
                val isImage = uri != null && (
                    mime?.startsWith("image/") == true ||
                        clip.description.hasMimeType("image/*")
                    )

                if (isImage && uri != null) {
                    Thread {
                        try {
                            val tmpDir = File(activity.cacheDir, "clipboard_tmp").apply { mkdirs() }
                            val tmpFile = File(tmpDir, "${UUID.randomUUID()}.img")
                            val maxBytes = 10L * 1024L * 1024L
                            var total = 0L
                            activity.contentResolver.openInputStream(uri)?.use { input ->
                                FileOutputStream(tmpFile).use { out ->
                                    val buf = ByteArray(64 * 1024)
                                    while (true) {
                                        val n = input.read(buf)
                                        if (n <= 0) break
                                        total += n
                                        if (total > maxBytes) {
                                            tmpFile.delete()
                                            invoke.reject("Clipboard image exceeds 10 MB limit")
                                            return@Thread
                                        }
                                        out.write(buf, 0, n)
                                    }
                                }
                            } ?: run {
                                invoke.reject("Unable to read clipboard image URI")
                                return@Thread
                            }
                            val res = JSObject()
                            res.put("kind", "image")
                            res.put("tempPath", tmpFile.absolutePath)
                            invoke.resolve(res)
                        } catch (ex: Exception) {
                            invoke.reject(ex.message ?: "Failed to read clipboard image")
                        }
                    }.start()
                    return@runOnUiThread
                }

                val text = item.coerceToText(activity)?.toString()
                if (!text.isNullOrBlank()) {
                    val res = JSObject()
                    res.put("kind", "text")
                    res.put("text", text)
                    invoke.resolve(res)
                } else {
                    val res = JSObject()
                    res.put("kind", "empty")
                    invoke.resolve(res)
                }
            } catch (ex: Exception) {
                invoke.reject(ex.message ?: "Failed to read system clipboard")
            }
        }
    }

    @Command
    fun writeSystemClipboardText(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(WriteClipboardTextArgs::class.java)
            activity.runOnUiThread {
                try {
                    val cm = activity.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager
                    if (cm == null) {
                        invoke.reject("ClipboardManager unavailable")
                        return@runOnUiThread
                    }
                    cm.setPrimaryClip(ClipData.newPlainText("Landa", args.text))
                    invoke.resolve()
                } catch (ex: Exception) {
                    invoke.reject(ex.message ?: "Failed to write clipboard text")
                }
            }
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: "Invalid clipboard text args")
        }
    }

    @Command
    fun writeSystemClipboardImage(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(WriteClipboardImageArgs::class.java)
            val source = File(args.imagePath).canonicalFile
            val dataRoot = activity.dataDir.canonicalFile
            val cacheRoot = activity.cacheDir.canonicalFile
            val filesRoot = activity.filesDir.canonicalFile
            val allowed = source.path.startsWith(dataRoot.path + File.separator) ||
                source.path.startsWith(cacheRoot.path + File.separator) ||
                source.path.startsWith(filesRoot.path + File.separator)
            if (!allowed || !source.exists() || !source.isFile) {
                invoke.reject("Invalid clipboard image path")
                return
            }

            val shareDir = File(activity.cacheDir, "clipboard_share").apply { mkdirs() }
            val cutoff = System.currentTimeMillis() - 60L * 60L * 1000L
            shareDir.listFiles()?.forEach { f ->
                if (f.isFile && f.lastModified() < cutoff) {
                    f.delete()
                }
            }

            val target = File(shareDir, "landa_${UUID.randomUUID().toString().take(8)}.png")
            FileInputStream(source).use { input ->
                FileOutputStream(target).use { out ->
                    input.copyTo(out, 64 * 1024)
                }
            }

            val uri = FileProvider.getUriForFile(
                activity,
                "${activity.packageName}.fileprovider",
                target
            )

            activity.runOnUiThread {
                try {
                    val cm = activity.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager
                    if (cm == null) {
                        invoke.reject("ClipboardManager unavailable")
                        return@runOnUiThread
                    }
                    cm.setPrimaryClip(ClipData.newUri(activity.contentResolver, "Landa Image", uri))
                    invoke.resolve()
                } catch (ex: Exception) {
                    invoke.reject(ex.message ?: "Failed to write clipboard image")
                }
            }
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: "Failed to copy image to clipboard")
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
    fun pickFolderForSend(invoke: Invoke) {
        try {
            val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).apply {
                addFlags(
                    Intent.FLAG_GRANT_READ_URI_PERMISSION or
                        Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION
                )
            }
            startActivityForResult(invoke, intent, "pickFolderForSendResult")
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: "Failed to open folder picker")
        }
    }

    @ActivityCallback
    fun pickFolderForSendResult(invoke: Invoke, result: ActivityResult) {
        if (result.resultCode == Activity.RESULT_CANCELED) {
            val res = JSObject()
            res.put("cancelled", true)
            res.put("items", JSArray())
            invoke.resolve(res)
            return
        }
        if (result.resultCode != Activity.RESULT_OK) {
            invoke.reject("Folder picker failed")
            return
        }
        val treeUri = result.data?.data
        if (treeUri == null) {
            val res = JSObject()
            res.put("cancelled", true)
            res.put("items", JSArray())
            invoke.resolve(res)
            return
        }

        runCatching {
            activity.contentResolver.takePersistableUriPermission(
                treeUri,
                Intent.FLAG_GRANT_READ_URI_PERMISSION
            )
        }

        Thread {
            try {
                val collected = collectSafTreeFiles(treeUri)
                val arr = JSArray()
                for (item in collected) {
                    val obj = JSObject()
                    obj.put("uri", item.uri)
                    obj.put("relativePath", item.relativePath)
                    obj.put("sizeBytes", item.sizeBytes)
                    arr.put(obj)
                }
                val res = JSObject()
                res.put("cancelled", false)
                res.put("items", arr)
                invoke.resolve(res)
            } catch (ex: Exception) {
                invoke.reject(ex.message ?: "Failed to read selected folder")
            }
        }.start()
    }

    @Command
    fun openContentUriFd(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(OpenContentUriArgs::class.java)
            if (!args.uri.startsWith("content://")) {
                invoke.reject("Only content:// URIs are supported")
                return
            }
            val parsed = Uri.parse(args.uri)
            val pfd = activity.contentResolver.openFileDescriptor(parsed, "r")
            if (pfd == null) {
                invoke.reject("Unable to open content URI")
                return
            }
            val fd = pfd.detachFd()
            val res = JSObject()
            res.put("fd", fd)
            invoke.resolve(res)
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: "Failed to open content URI")
        }
    }

    private fun queryContentUriNameAndRelPath(parsed: Uri): Pair<String?, String?> {
        var name: String? = null
        var relativePath: String? = null
        val isMediaStore = Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q &&
            parsed.authority == MediaStore.AUTHORITY
        val baseProjection = arrayOf(OpenableColumns.DISPLAY_NAME)
        val extProjection = if (isMediaStore) {
            arrayOf(OpenableColumns.DISPLAY_NAME, MediaStore.MediaColumns.RELATIVE_PATH)
        } else {
            baseProjection
        }

        val queried = runCatching {
            activity.contentResolver.query(parsed, extProjection, null, null, null)?.use { cursor ->
                if (cursor.moveToFirst()) {
                    val nameIdx = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                    if (nameIdx >= 0 && !cursor.isNull(nameIdx)) {
                        name = cursor.getString(nameIdx)
                    }
                    if (isMediaStore) {
                        val relIdx = cursor.getColumnIndex(MediaStore.MediaColumns.RELATIVE_PATH)
                        if (relIdx >= 0 && !cursor.isNull(relIdx)) {
                            relativePath = cursor.getString(relIdx)
                        }
                    }
                }
            }
        }
        if (queried.isFailure && isMediaStore) {
            runCatching {
                activity.contentResolver.query(parsed, baseProjection, null, null, null)?.use { cursor ->
                    if (cursor.moveToFirst()) {
                        val nameIdx = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                        if (nameIdx >= 0 && !cursor.isNull(nameIdx)) {
                            name = cursor.getString(nameIdx)
                        }
                    }
                }
            }
        }
        return Pair(name, relativePath)
    }

    @Command
    fun getContentUriMeta(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(OpenContentUriArgs::class.java)
            if (!args.uri.startsWith("content://")) {
                invoke.reject("Only content:// URIs are supported")
                return
            }
            val parsed = Uri.parse(args.uri)
            var name: String? = null
            var sizeBytes = -1L
            var relativePath: String? = null
            val isMediaStore = Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q &&
                parsed.authority == MediaStore.AUTHORITY
            val baseProjection = arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE)
            val extProjection = if (isMediaStore) {
                arrayOf(
                    OpenableColumns.DISPLAY_NAME,
                    OpenableColumns.SIZE,
                    MediaStore.MediaColumns.RELATIVE_PATH
                )
            } else {
                baseProjection
            }

            val queryRes = runCatching {
                activity.contentResolver.query(
                    parsed,
                    extProjection,
                    null,
                    null,
                    null
                )?.use { cursor ->
                    if (cursor.moveToFirst()) {
                        val nameIdx = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                        val sizeIdx = cursor.getColumnIndex(OpenableColumns.SIZE)
                        if (nameIdx >= 0 && !cursor.isNull(nameIdx)) {
                            name = cursor.getString(nameIdx)
                        }
                        if (sizeIdx >= 0 && !cursor.isNull(sizeIdx)) {
                            sizeBytes = cursor.getLong(sizeIdx)
                        }
                        if (isMediaStore) {
                            val relIdx = cursor.getColumnIndex(MediaStore.MediaColumns.RELATIVE_PATH)
                            if (relIdx >= 0 && !cursor.isNull(relIdx)) {
                                relativePath = cursor.getString(relIdx)
                            }
                        }
                    }
                }
            }
            if (queryRes.isFailure) {
                if (isMediaStore) {
                    activity.contentResolver.query(
                        parsed,
                        baseProjection,
                        null,
                        null,
                        null
                    )?.use { cursor ->
                        if (cursor.moveToFirst()) {
                            val nameIdx = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                            val sizeIdx = cursor.getColumnIndex(OpenableColumns.SIZE)
                            if (nameIdx >= 0 && !cursor.isNull(nameIdx)) {
                                name = cursor.getString(nameIdx)
                            }
                            if (sizeIdx >= 0 && !cursor.isNull(sizeIdx)) {
                                sizeBytes = cursor.getLong(sizeIdx)
                            }
                        }
                    }
                } else {
                    queryRes.getOrThrow()
                }
            }
            if (sizeBytes <= 0L) {
                sizeBytes = runCatching {
                    activity.contentResolver.openFileDescriptor(parsed, "r")?.use { it.statSize } ?: 0L
                }.getOrDefault(0L)
            }
            val mimeType = activity.contentResolver.getType(parsed) ?: "application/octet-stream"
            val cleanName = name?.trim()?.ifEmpty { null } ?: "file"
            val displayPath = DownloadsPublisher.formatResolvedMediaStorePath(relativePath, name)
                ?: publishedDisplayPaths[args.uri]
            val res = JSObject()
            res.put("name", cleanName)
            res.put("sizeBytes", sizeBytes.coerceAtLeast(0L))
            res.put("mimeType", mimeType)
            if (displayPath != null) {
                res.put("displayPath", displayPath)
            }
            invoke.resolve(res)
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: "Failed to query content URI metadata")
        }
    }

    @Command
    fun resolveContentUriDisplayNames(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(ResolveContentUriDisplayNamesArgs::class.java)
            val names = JSObject()
            val seen = HashSet<String>()
            for (rawUri in args.uris) {
                if (!rawUri.startsWith("content://") || !seen.add(rawUri)) continue
                val parsed = Uri.parse(rawUri)
                val (name, relativePath) = queryContentUriNameAndRelPath(parsed)
                val resolved = DownloadsPublisher.formatResolvedMediaStorePath(relativePath, name)
                    ?: publishedDisplayPaths[rawUri]
                    ?: name?.trim()?.ifEmpty { null }
                if (resolved != null) {
                    names.put(rawUri, resolved)
                }
            }
            val res = JSObject()
            res.put("names", names)
            invoke.resolve(res)
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: "Failed to resolve content URI display names")
        }
    }

    @Command
    fun resolveContentUriForPath(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(OpenContentUriArgs::class.java)
            val target = args.uri.trim()
            val resolved = if (target.isEmpty()) null else resolveContentUriForPathInternal(target)
            val res = JSObject()
            res.put("uri", resolved?.toString())
            invoke.resolve(res)
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: "Failed to resolve content URI")
        }
    }

    private fun resolveContentUriForPathInternal(path: String): Uri? {
        publishedUris[path]?.first?.let { return it }
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) {
            return null
        }
        val resolver = activity.contentResolver
        val collection = MediaStore.Downloads.EXTERNAL_CONTENT_URI
        val projection = arrayOf(MediaStore.MediaColumns._ID)

        runCatching {
            resolver.query(
                collection,
                projection,
                "${MediaStore.MediaColumns.DATA} = ?",
                arrayOf(path),
                "${MediaStore.MediaColumns.DATE_ADDED} DESC"
            )?.use { cursor ->
                if (cursor.moveToFirst()) {
                    val idIdx = cursor.getColumnIndex(MediaStore.MediaColumns._ID)
                    if (idIdx >= 0) {
                        return ContentUris.withAppendedId(collection, cursor.getLong(idIdx))
                    }
                }
            }
        }

        val relQuery = DownloadsPublisher.parseDownloadRelativeQuery(path) ?: return null
        val (relDir, displayName) = relQuery
        return runCatching {
            resolver.query(
                collection,
                projection,
                "${MediaStore.MediaColumns.RELATIVE_PATH} = ? AND ${MediaStore.MediaColumns.DISPLAY_NAME} = ?",
                arrayOf(relDir, displayName),
                "${MediaStore.MediaColumns.DATE_ADDED} DESC"
            )?.use { cursor ->
                if (cursor.moveToFirst()) {
                    val idIdx = cursor.getColumnIndex(MediaStore.MediaColumns._ID)
                    if (idIdx >= 0) {
                        ContentUris.withAppendedId(collection, cursor.getLong(idIdx))
                    } else null
                } else null
            }
        }.getOrNull()
    }

    @Command
    fun openFileExternal(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(OpenContentUriArgs::class.java)
            val target = args.uri.trim()
            if (target.isEmpty()) {
                invoke.reject("Empty path")
                return
            }
            val intent = if (target.startsWith("content://")) {
                val parsed = Uri.parse(target)
                val mime = activity.contentResolver.getType(parsed) ?: "*/*"
                Intent(Intent.ACTION_VIEW).apply {
                    setDataAndType(parsed, mime)
                    addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
                }
            } else {
                val cached = publishedUris[target]
                if (cached != null) {
                    Intent(Intent.ACTION_VIEW).apply {
                        setDataAndType(cached.first, cached.second)
                        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
                    }
                } else {
                    val file = File(target).canonicalFile
                    if (!file.exists() || !file.isFile || !file.canRead()) {
                        val resolvedUri = resolveContentUriForPathInternal(target)
                        if (resolvedUri == null) {
                            invoke.reject("File not found")
                            return
                        }
                        val mime = activity.contentResolver.getType(resolvedUri) ?: "*/*"
                        Intent(Intent.ACTION_VIEW).apply {
                            setDataAndType(resolvedUri, mime)
                            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
                        }
                    } else {
                        val uri = FileProvider.getUriForFile(
                            activity,
                            "${activity.packageName}.fileprovider",
                            file
                        )
                        val ext = file.extension.lowercase()
                        val mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext)
                            ?: if (ext == "log" || ext == "txt") "text/plain" else "*/*"
                        Intent(Intent.ACTION_VIEW).apply {
                            setDataAndType(uri, mime)
                            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
                        }
                    }
                }
            }
            activity.startActivity(intent)
            invoke.resolve()
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: "No app found to open this file")
        }
    }

    private data class SafFileEntry(
        val uri: String,
        val relativePath: String,
        val sizeBytes: Long
    )

    private fun collectSafTreeFiles(treeUri: Uri): List<SafFileEntry> {
        val rootDocId = DocumentsContract.getTreeDocumentId(treeUri)
        val rootDocUri = DocumentsContract.buildDocumentUriUsingTree(treeUri, rootDocId)
        val rawRootName = queryDisplayName(rootDocUri) ?: "folder"
        val rootName = sanitizeSegment(rawRootName).ifEmpty { "folder" }

        val out = mutableListOf<SafFileEntry>()
        walkSafDirectory(treeUri, rootDocId, rootName, 0, out)
        return out
    }

    private fun walkSafDirectory(
        treeUri: Uri,
        parentDocId: String,
        prefixPath: String,
        depth: Int,
        out: MutableList<SafFileEntry>
    ) {
        if (depth > MAX_FOLDER_DEPTH || out.size >= MAX_FOLDER_FILES) {
            return
        }

        val childrenUri = DocumentsContract.buildChildDocumentsUriUsingTree(treeUri, parentDocId)
        val projection = arrayOf(
            DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME,
            DocumentsContract.Document.COLUMN_MIME_TYPE,
            DocumentsContract.Document.COLUMN_SIZE,
            DocumentsContract.Document.COLUMN_FLAGS
        )

        activity.contentResolver.query(childrenUri, projection, null, null, null)?.use { cursor ->
            val idIdx = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_DOCUMENT_ID)
            val nameIdx = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_DISPLAY_NAME)
            val mimeIdx = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_MIME_TYPE)
            val sizeIdx = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_SIZE)
            val flagsIdx = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_FLAGS)

            while (cursor.moveToNext() && out.size < MAX_FOLDER_FILES) {
                if (idIdx < 0) continue
                val docId = cursor.getString(idIdx) ?: continue
                val rawName = if (nameIdx >= 0) cursor.getString(nameIdx) else null
                val safeName = sanitizeSegment(rawName ?: "item")
                if (safeName.isEmpty()) continue

                val mime = if (mimeIdx >= 0) cursor.getString(mimeIdx) else null
                val flags = if (flagsIdx >= 0 && !cursor.isNull(flagsIdx)) cursor.getInt(flagsIdx) else 0
                val isVirtual = (flags and DocumentsContract.Document.FLAG_VIRTUAL_DOCUMENT) != 0
                if (isVirtual) continue

                val nextRel = "$prefixPath/$safeName"
                if (mime == DocumentsContract.Document.MIME_TYPE_DIR) {
                    walkSafDirectory(treeUri, docId, nextRel, depth + 1, out)
                } else {
                    val docUri = DocumentsContract.buildDocumentUriUsingTree(treeUri, docId)
                    var size = if (sizeIdx >= 0 && !cursor.isNull(sizeIdx)) {
                        cursor.getLong(sizeIdx)
                    } else {
                        -1L
                    }
                    if (size < 0L) {
                        size = runCatching {
                            activity.contentResolver.openFileDescriptor(docUri, "r")?.use { it.statSize } ?: -1L
                        }.getOrDefault(-1L)
                    }
                    if (size < 0L) continue
                    out.add(
                        SafFileEntry(
                            uri = docUri.toString(),
                            relativePath = nextRel,
                            sizeBytes = size
                        )
                    )
                }
            }
        }
    }

    private fun sanitizeSegment(raw: String): String {
        val cleaned = raw.trim()
            .replace('/', '_')
            .replace('\\', '_')
            .replace('\u0000', '_')
        return if (cleaned == "." || cleaned == "..") "" else cleaned
    }

    @Command
    fun pickShareFolder(invoke: Invoke) {
        try {
            val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).apply {
                addFlags(
                    Intent.FLAG_GRANT_READ_URI_PERMISSION or
                        Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION or
                        Intent.FLAG_GRANT_PREFIX_URI_PERMISSION
                )
            }
            startActivityForResult(invoke, intent, "pickShareFolderResult")
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: "Failed to open folder picker")
        }
    }

    @ActivityCallback
    fun pickShareFolderResult(invoke: Invoke, result: ActivityResult) {
        if (result.resultCode == Activity.RESULT_CANCELED) {
            val res = JSObject()
            res.put("cancelled", true)
            invoke.resolve(res)
            return
        }
        if (result.resultCode != Activity.RESULT_OK) {
            invoke.reject("Folder picker failed")
            return
        }
        val treeUri = result.data?.data
        if (treeUri == null) {
            val res = JSObject()
            res.put("cancelled", true)
            invoke.resolve(res)
            return
        }

        try {
            activity.contentResolver.takePersistableUriPermission(
                treeUri,
                Intent.FLAG_GRANT_READ_URI_PERMISSION
            )
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: "Failed to persist folder permission")
            return
        }

        val rootDocUri = DocumentsContract.buildDocumentUriUsingTree(
            treeUri,
            DocumentsContract.getTreeDocumentId(treeUri)
        )
        val name = queryDisplayName(rootDocUri)?.trim()?.ifEmpty { null } ?: "Folder"
        val res = JSObject()
        res.put("cancelled", false)
        res.put("uri", treeUri.toString())
        res.put("name", name)
        res.put("isDir", true)
        invoke.resolve(res)
    }

    @Command
    fun pickShareFile(invoke: Invoke) {
        try {
            val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
                addCategory(Intent.CATEGORY_OPENABLE)
                type = "*/*"
                addFlags(
                    Intent.FLAG_GRANT_READ_URI_PERMISSION or
                        Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION
                )
            }
            startActivityForResult(invoke, intent, "pickShareFileResult")
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: "Failed to open file picker")
        }
    }

    @ActivityCallback
    fun pickShareFileResult(invoke: Invoke, result: ActivityResult) {
        if (result.resultCode == Activity.RESULT_CANCELED) {
            val res = JSObject()
            res.put("cancelled", true)
            invoke.resolve(res)
            return
        }
        if (result.resultCode != Activity.RESULT_OK) {
            invoke.reject("File picker failed")
            return
        }
        val fileUri = result.data?.data
        if (fileUri == null) {
            val res = JSObject()
            res.put("cancelled", true)
            invoke.resolve(res)
            return
        }

        try {
            activity.contentResolver.takePersistableUriPermission(
                fileUri,
                Intent.FLAG_GRANT_READ_URI_PERMISSION
            )
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: "Failed to persist file permission")
            return
        }

        val name = queryDisplayName(fileUri)?.trim()?.ifEmpty { null } ?: "File"
        val res = JSObject()
        res.put("cancelled", false)
        res.put("uri", fileUri.toString())
        res.put("name", name)
        res.put("isDir", false)
        invoke.resolve(res)
    }

    @Command
    fun listSafShareDir(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(ListSafShareDirArgs::class.java)
            if (!args.rootUri.startsWith("content://")) {
                invoke.reject("Only content:// URIs are supported")
                return
            }
            val rootUri = Uri.parse(args.rootUri)
            val segments = parseSafeRelSegments(args.relPath)

            if (!args.isDir) {
                if (segments.isNotEmpty()) {
                    invoke.reject("cannot traverse into file share")
                    return
                }
                val meta = queryDocumentMeta(rootUri)
                val arr = JSArray()
                val obj = JSObject()
                obj.put("name", meta.name)
                obj.put("relPath", "")
                obj.put("isDir", false)
                obj.put("sizeBytes", meta.sizeBytes)
                obj.put("modifiedMs", meta.modifiedMs)
                arr.put(obj)
                val res = JSObject()
                res.put("entries", arr)
                invoke.resolve(res)
                return
            }

            val targetDocId = resolveSafTreeDocId(rootUri, segments, expectDir = true)
            val childrenUri = DocumentsContract.buildChildDocumentsUriUsingTree(rootUri, targetDocId)
            val projection = arrayOf(
                DocumentsContract.Document.COLUMN_DOCUMENT_ID,
                DocumentsContract.Document.COLUMN_DISPLAY_NAME,
                DocumentsContract.Document.COLUMN_MIME_TYPE,
                DocumentsContract.Document.COLUMN_SIZE,
                DocumentsContract.Document.COLUMN_LAST_MODIFIED,
                DocumentsContract.Document.COLUMN_FLAGS
            )

            data class EntryRow(
                val name: String,
                val relPath: String,
                val isDir: Boolean,
                val sizeBytes: Long,
                val modifiedMs: Long
            )

            val rows = mutableListOf<EntryRow>()
            val prefix = segments.joinToString("/")
            activity.contentResolver.query(childrenUri, projection, null, null, null)?.use { cursor ->
                val nameIdx = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_DISPLAY_NAME)
                val mimeIdx = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_MIME_TYPE)
                val sizeIdx = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_SIZE)
                val modIdx = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_LAST_MODIFIED)
                val flagsIdx = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_FLAGS)

                while (cursor.moveToNext() && rows.size < 5000) {
                    val rawName = if (nameIdx >= 0) cursor.getString(nameIdx) else null
                    val name = rawName?.trim().orEmpty()
                    if (name.isEmpty() || name == "." || name == ".." ||
                        name.contains('/') || name.contains('\\') || name.contains('\u0000')
                    ) {
                        continue
                    }
                    val flags = if (flagsIdx >= 0 && !cursor.isNull(flagsIdx)) cursor.getInt(flagsIdx) else 0
                    if ((flags and DocumentsContract.Document.FLAG_VIRTUAL_DOCUMENT) != 0) {
                        continue
                    }
                    val mime = if (mimeIdx >= 0) cursor.getString(mimeIdx) else null
                    val isDir = mime == DocumentsContract.Document.MIME_TYPE_DIR
                    val sizeBytes = if (!isDir && sizeIdx >= 0 && !cursor.isNull(sizeIdx)) {
                        cursor.getLong(sizeIdx).coerceAtLeast(0L)
                    } else {
                        0L
                    }
                    val modifiedMs = if (modIdx >= 0 && !cursor.isNull(modIdx)) {
                        cursor.getLong(modIdx).coerceAtLeast(0L)
                    } else {
                        0L
                    }
                    val nextRel = if (prefix.isEmpty()) name else "$prefix/$name"
                    rows.add(EntryRow(name, nextRel, isDir, sizeBytes, modifiedMs))
                }
            }

            rows.sortWith(compareByDescending<EntryRow> { it.isDir }.thenBy { it.name })
            val arr = JSArray()
            for (row in rows) {
                val obj = JSObject()
                obj.put("name", row.name)
                obj.put("relPath", row.relPath)
                obj.put("isDir", row.isDir)
                obj.put("sizeBytes", row.sizeBytes)
                obj.put("modifiedMs", row.modifiedMs)
                arr.put(obj)
            }
            val res = JSObject()
            res.put("entries", arr)
            invoke.resolve(res)
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: "Failed to list shared folder")
        }
    }

    @Command
    fun listSafShareTree(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(ListSafShareTreeArgs::class.java)
            if (!args.rootUri.startsWith("content://")) {
                invoke.reject("Only content:// URIs are supported")
                return
            }
            if (!args.isDir) {
                invoke.reject("target is not a directory")
                return
            }
            val rootUri = Uri.parse(args.rootUri)
            val segments = parseSafeRelSegments(args.relPath)
            val maxDepth = args.maxDepth.coerceIn(1, MAX_FOLDER_DEPTH)
            val maxFiles = args.maxFiles.coerceIn(1, MAX_FOLDER_FILES)

            val startDocId = resolveSafTreeDocId(rootUri, segments, expectDir = true)
            val startPrefix = segments.joinToString("/")

            val projection = arrayOf(
                DocumentsContract.Document.COLUMN_DOCUMENT_ID,
                DocumentsContract.Document.COLUMN_DISPLAY_NAME,
                DocumentsContract.Document.COLUMN_MIME_TYPE,
                DocumentsContract.Document.COLUMN_SIZE,
                DocumentsContract.Document.COLUMN_LAST_MODIFIED,
                DocumentsContract.Document.COLUMN_FLAGS
            )

            data class TreeRow(
                val name: String,
                val relPath: String,
                val isDir: Boolean,
                val sizeBytes: Long,
                val modifiedMs: Long
            )

            val rows = mutableListOf<TreeRow>()
            val seenRelPaths = HashSet<String>()
            var dirCount = 0
            var fileCount = 0
            val stack = ArrayDeque<Triple<String, String, Int>>()
            stack.addLast(Triple(startDocId, startPrefix, 0))

            while (stack.isNotEmpty()) {
                val (docId, prefix, depth) = stack.removeLast()
                val childrenUri = DocumentsContract.buildChildDocumentsUriUsingTree(rootUri, docId)
                activity.contentResolver.query(childrenUri, projection, null, null, null)?.use { cursor ->
                    val idIdx = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_DOCUMENT_ID)
                    val nameIdx = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_DISPLAY_NAME)
                    val mimeIdx = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_MIME_TYPE)
                    val sizeIdx = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_SIZE)
                    val modIdx = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_LAST_MODIFIED)
                    val flagsIdx = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_FLAGS)

                    while (cursor.moveToNext()) {
                        if (idIdx < 0) continue
                        val childDocId = cursor.getString(idIdx) ?: continue
                        val rawName = if (nameIdx >= 0) cursor.getString(nameIdx) else null
                        val name = rawName?.trim().orEmpty()
                        if (name.isEmpty() || name == "." || name == ".." ||
                            name.contains('/') || name.contains('\\') || name.contains('\u0000')
                        ) {
                            continue
                        }
                        val flags = if (flagsIdx >= 0 && !cursor.isNull(flagsIdx)) cursor.getInt(flagsIdx) else 0
                        if ((flags and DocumentsContract.Document.FLAG_VIRTUAL_DOCUMENT) != 0) {
                            continue
                        }
                        val mime = if (mimeIdx >= 0) cursor.getString(mimeIdx) else null
                        val isDir = mime == DocumentsContract.Document.MIME_TYPE_DIR
                        val nextRel = if (prefix.isEmpty()) name else "$prefix/$name"
                        if (!seenRelPaths.add(nextRel)) {
                            invoke.reject("duplicate path in shared folder")
                            return
                        }
                        val modifiedMs = if (modIdx >= 0 && !cursor.isNull(modIdx)) {
                            cursor.getLong(modIdx).coerceAtLeast(0L)
                        } else {
                            0L
                        }

                        if (isDir) {
                            if (depth >= maxDepth) {
                                invoke.reject("folder exceeds maximum depth ($maxDepth)")
                                return
                            }
                            if (dirCount >= maxFiles) {
                                invoke.reject("folder exceeds maximum directory count ($maxFiles)")
                                return
                            }
                            dirCount++
                            rows.add(TreeRow(name, nextRel, true, 0L, modifiedMs))
                            stack.addLast(Triple(childDocId, nextRel, depth + 1))
                        } else {
                            if (fileCount >= maxFiles) {
                                invoke.reject("folder exceeds maximum file count ($maxFiles)")
                                return
                            }
                            var sizeBytes = if (sizeIdx >= 0 && !cursor.isNull(sizeIdx)) {
                                cursor.getLong(sizeIdx)
                            } else {
                                -1L
                            }
                            if (sizeBytes < 0L) {
                                val docUri = DocumentsContract.buildDocumentUriUsingTree(rootUri, childDocId)
                                sizeBytes = runCatching {
                                    activity.contentResolver.openFileDescriptor(docUri, "r")?.use { it.statSize } ?: -1L
                                }.getOrDefault(-1L)
                            }
                            if (sizeBytes < 0L) continue
                            fileCount++
                            rows.add(TreeRow(name, nextRel, false, sizeBytes, modifiedMs))
                        }
                    }
                }
            }

            rows.sortBy { it.relPath }
            val arr = JSArray()
            for (row in rows) {
                val obj = JSObject()
                obj.put("name", row.name)
                obj.put("relPath", row.relPath)
                obj.put("isDir", row.isDir)
                obj.put("sizeBytes", row.sizeBytes)
                obj.put("modifiedMs", row.modifiedMs)
                arr.put(obj)
            }
            val res = JSObject()
            res.put("entries", arr)
            invoke.resolve(res)
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: "Failed to traverse shared folder")
        }
    }

    @Command
    fun openSafShareFile(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(OpenSafShareFileArgs::class.java)
            if (!args.rootUri.startsWith("content://")) {
                invoke.reject("Only content:// URIs are supported")
                return
            }
            val rootUri = Uri.parse(args.rootUri)
            val segments = parseSafeRelSegments(args.relPath)

            val targetUri = if (!args.isDir) {
                if (segments.isNotEmpty()) {
                    invoke.reject("cannot traverse into file share")
                    return
                }
                rootUri
            } else {
                if (segments.isEmpty()) {
                    invoke.reject("target is not a file")
                    return
                }
                val docId = resolveSafTreeDocId(rootUri, segments, expectDir = false)
                DocumentsContract.buildDocumentUriUsingTree(rootUri, docId)
            }

            val meta = queryDocumentMeta(targetUri)
            val pfd = activity.contentResolver.openFileDescriptor(targetUri, "r")
            if (pfd == null) {
                invoke.reject("Unable to open shared file")
                return
            }
            val statSize = runCatching { pfd.statSize }.getOrDefault(-1L)
            val sizeBytes = if (meta.sizeBytes > 0L) meta.sizeBytes else statSize.coerceAtLeast(0L)
            val fd = pfd.detachFd()

            val res = JSObject()
            res.put("fd", fd)
            res.put("fileName", meta.name)
            res.put("sizeBytes", sizeBytes)
            res.put("modifiedMs", meta.modifiedMs)
            invoke.resolve(res)
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: "Failed to open shared file")
        }
    }

    private data class SafDocMeta(
        val name: String,
        val sizeBytes: Long,
        val modifiedMs: Long
    )

    private fun parseSafeRelSegments(relPath: String): List<String> {
        val trimmed = relPath.trim()
        if (trimmed.isEmpty() || trimmed == "." || trimmed == "/") {
            return emptyList()
        }
        if (trimmed.contains('\\') || trimmed.contains('\u0000')) {
            throw IllegalArgumentException("Invalid relative path")
        }
        val parts = trimmed.split('/').filter { it.isNotEmpty() }
        for (part in parts) {
            if (part == "." || part == "..") {
                throw IllegalArgumentException("Path traversal is not allowed")
            }
        }
        return parts
    }

    private fun resolveSafTreeDocId(
        treeUri: Uri,
        segments: List<String>,
        expectDir: Boolean
    ): String {
        var currentDocId = DocumentsContract.getTreeDocumentId(treeUri)
        val projection = arrayOf(
            DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME,
            DocumentsContract.Document.COLUMN_MIME_TYPE,
            DocumentsContract.Document.COLUMN_FLAGS
        )

        for ((idx, seg) in segments.withIndex()) {
            val isLast = idx == segments.lastIndex
            val childrenUri = DocumentsContract.buildChildDocumentsUriUsingTree(treeUri, currentDocId)
            var matchedDocId: String? = null
            var matchedIsDir = false

            activity.contentResolver.query(childrenUri, projection, null, null, null)?.use { cursor ->
                val idIdx = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_DOCUMENT_ID)
                val nameIdx = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_DISPLAY_NAME)
                val mimeIdx = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_MIME_TYPE)
                val flagsIdx = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_FLAGS)

                while (cursor.moveToNext()) {
                    if (idIdx < 0 || nameIdx < 0) continue
                    val flags = if (flagsIdx >= 0 && !cursor.isNull(flagsIdx)) cursor.getInt(flagsIdx) else 0
                    if ((flags and DocumentsContract.Document.FLAG_VIRTUAL_DOCUMENT) != 0) {
                        continue
                    }
                    val name = cursor.getString(nameIdx)?.trim() ?: continue
                    if (name == seg) {
                        matchedDocId = cursor.getString(idIdx)
                        val mime = if (mimeIdx >= 0) cursor.getString(mimeIdx) else null
                        matchedIsDir = mime == DocumentsContract.Document.MIME_TYPE_DIR
                        break
                    }
                }
            }

            val nextId = matchedDocId ?: throw IllegalArgumentException("Path not found in share")
            if (!isLast && !matchedIsDir) {
                throw IllegalArgumentException("Intermediate path segment is not a directory")
            }
            if (isLast) {
                if (expectDir && !matchedIsDir) {
                    throw IllegalArgumentException("Target is not a directory")
                }
                if (!expectDir && matchedIsDir) {
                    throw IllegalArgumentException("target is not a file")
                }
            }
            currentDocId = nextId
        }

        return currentDocId
    }

    private fun queryDocumentMeta(uri: Uri): SafDocMeta {
        var name: String? = null
        var sizeBytes = 0L
        var modifiedMs = 0L
        activity.contentResolver.query(
            uri,
            arrayOf(
                OpenableColumns.DISPLAY_NAME,
                OpenableColumns.SIZE,
                DocumentsContract.Document.COLUMN_LAST_MODIFIED
            ),
            null,
            null,
            null
        )?.use { cursor ->
            if (cursor.moveToFirst()) {
                val nameIdx = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                val sizeIdx = cursor.getColumnIndex(OpenableColumns.SIZE)
                val modIdx = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_LAST_MODIFIED)
                if (nameIdx >= 0) name = cursor.getString(nameIdx)
                if (sizeIdx >= 0 && !cursor.isNull(sizeIdx)) {
                    sizeBytes = cursor.getLong(sizeIdx).coerceAtLeast(0L)
                }
                if (modIdx >= 0 && !cursor.isNull(modIdx)) {
                    modifiedMs = cursor.getLong(modIdx).coerceAtLeast(0L)
                }
            }
        }
        return SafDocMeta(
            name = name?.trim()?.ifEmpty { null } ?: "file.bin",
            sizeBytes = sizeBytes,
            modifiedMs = modifiedMs
        )
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
                try {
                    activity.startActivity(settingsIntent)
                } catch (_: Exception) {
                    val fallbackIntent = Intent(Settings.ACTION_SECURITY_SETTINGS).apply {
                        addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                    }
                    activity.startActivity(fallbackIntent)
                }
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
            val published = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                publishViaMediaStore(sourceFile, cleanRel)
            } else {
                publishLegacyDownloads(sourceFile, cleanRel)
            }

            sourceFile.delete()

            val res = JSObject()
            res.put("publishedPath", published.publishedPath)
            invoke.resolve(res)
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: "Failed to publish file to Downloads")
        }
    }

    private fun publishViaMediaStore(sourceFile: File, cleanRel: String): PublishedDownload {
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
            var actualDisplayName: String? = null
            resolver.query(
                uri,
                arrayOf(MediaStore.MediaColumns.DATA, MediaStore.MediaColumns.DISPLAY_NAME),
                null,
                null,
                null
            )?.use { cursor ->
                if (cursor.moveToFirst()) {
                    val dataIdx = cursor.getColumnIndex(MediaStore.MediaColumns.DATA)
                    if (dataIdx >= 0 && !cursor.isNull(dataIdx)) {
                        resolvedPath = cursor.getString(dataIdx)
                    }
                    val nameIdx = cursor.getColumnIndex(MediaStore.MediaColumns.DISPLAY_NAME)
                    if (nameIdx >= 0 && !cursor.isNull(nameIdx)) {
                        actualDisplayName = cursor.getString(nameIdx)
                    }
                }
            }
            val contentUriStr = uri.toString()
            val publishedPath = DownloadsPublisher.selectMediaStorePublishedPath(
                contentUri = contentUriStr,
                resolvedPath = resolvedPath,
            )
            publishedUris[contentUriStr] = Pair(uri, mimeType)
            resolvedPath?.let { publishedUris[it] = Pair(uri, mimeType) }
            val displayPath = DownloadsPublisher.formatMediaStoreDisplayPath(
                parts = parts,
                actualDisplayName = actualDisplayName,
                resolvedPath = resolvedPath,
                fallbackFileName = fileName,
            )
            publishedDisplayPaths[contentUriStr] = displayPath
            return PublishedDownload(
                publishedPath = publishedPath,
                displayPath = displayPath,
            )
        } catch (ex: Exception) {
            resolver.delete(uri, null, null)
            throw ex
        }
    }

    @Suppress("DEPRECATION")
    private fun publishLegacyDownloads(sourceFile: File, cleanRel: String): PublishedDownload {
        val downloadsRoot = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS)
        val targetBase = File(downloadsRoot, "Landa")
        val target = DownloadsPublisher.publishLegacyFile(targetBase, sourceFile, cleanRel)
        val ext = target.extension.lowercase()
        val mimeType = MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext)
            ?: "application/octet-stream"
        val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.fileprovider", target)
        publishedUris[target.absolutePath] = Pair(uri, mimeType)
        return PublishedDownload(
            publishedPath = target.absolutePath,
            displayPath = DownloadsPublisher.formatLegacyDisplayPath(targetBase, target),
        )
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

    private fun stopTransferForegroundInternal() {
        lastFgUpdateMs = 0L
        lastFgActiveCount = 0
        fgServiceRunning = false
        try {
            activity.stopService(Intent(activity, TransferForegroundService::class.java))
        } catch (_: Exception) {
        }
        try {
            val nm = activity.getSystemService(Context.NOTIFICATION_SERVICE) as? NotificationManager
            nm?.cancel(TransferForegroundService.FG_NOTIFICATION_ID)
        } catch (_: Exception) {
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
        if (nm.getNotificationChannel(CHANNEL_REQUESTS) == null) {
            val ch = NotificationChannel(
                CHANNEL_REQUESTS,
                "Incoming Requests",
                NotificationManager.IMPORTANCE_HIGH
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

    private fun offerNotifId(transferId: String): Int {
        return 30000 + (transferId.hashCode() and 0x7FFF)
    }

    private fun friendNotifId(peerId: String): Int {
        return 40000 + (peerId.hashCode() and 0x7FFF)
    }

    private fun formatBytesCompact(bytes: Long): String {
        if (bytes < 1024L) return "$bytes B"
        val units = arrayOf("KB", "MB", "GB", "TB")
        var value = bytes.toDouble() / 1024.0
        var idx = 0
        while (value >= 1024.0 && idx < units.size - 1) {
            value /= 1024.0
            idx++
        }
        return if (value >= 100.0) {
            "${value.toInt()} ${units[idx]}"
        } else {
            String.format(java.util.Locale.US, "%.1f %s", value, units[idx])
        }
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

    private fun runOnMainThread(block: () -> Unit) {
        if (Looper.myLooper() == Looper.getMainLooper()) {
            block()
        } else {
            mainHandler.post(block)
        }
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
                    runOnMainThread {
                        if (isDestroyed) return@runOnMainThread
                        acquireMulticastLock(forceReacquire = true)
                        if (appInForeground) {
                            acquireWifiLock()
                        }
                        triggerNetworkChanged(NetworkChangeSource.NETWORK_CALLBACK)
                    }
                }

                override fun onLost(network: Network) {
                    runOnMainThread {
                        if (isDestroyed) return@runOnMainThread
                        acquireMulticastLock(forceReacquire = true)
                        triggerNetworkChanged(NetworkChangeSource.NETWORK_CALLBACK)
                    }
                }
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                cm.registerNetworkCallback(req, cb, mainHandler)
            } else {
                cm.registerNetworkCallback(req, cb)
            }
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

    private fun triggerNetworkChanged(source: NetworkChangeSource) {
        NetworkChangedDispatcher.dispatch(
            source = source,
            hasListener = hasListener("network-changed"),
            setPending = { pendingNetworkChanged = it },
            notifyNativeListener = { trigger("network-changed", JSObject()) },
            notifyWebView = { dispatchWebEvent("landa-network-changed") },
        )
    }

    private fun dispatchWebEvent(eventName: String) {
        activity.runOnUiThread {
            webViewRef?.evaluateJavascript(
                "window.dispatchEvent(new Event('$eventName'))",
                null
            )
        }
    }

    private fun acquireMulticastLock(forceReacquire: Boolean = false) {
        try {
            if (forceReacquire && multicastLock?.isHeld == true) {
                runCatching { multicastLock?.release() }
                multicastLock = null
            }
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

    private fun acquireWifiLock() {
        try {
            if (wifiLock?.isHeld == true) return
            val wifi = activity.applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager
                ?: return
            @Suppress("DEPRECATION")
            val mode = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                WifiManager.WIFI_MODE_FULL_LOW_LATENCY
            } else {
                WifiManager.WIFI_MODE_FULL_HIGH_PERF
            }
            wifiLock = wifi.createWifiLock(mode, "landa:foreground_wifi").apply {
                setReferenceCounted(false)
                acquire()
            }
        } catch (_: Exception) {
        }
    }

    private fun releaseWifiLock() {
        try {
            if (wifiLock?.isHeld == true) {
                wifiLock?.release()
            }
        } catch (_: Exception) {
        } finally {
            wifiLock = null
        }
    }

    private fun handleFocusIntent(intent: Intent) {
        val target = intent.getStringExtra("landa_focus_target") ?: return
        val transferId = intent.getStringExtra("landa_transfer_id")
        val peerId = intent.getStringExtra("landa_peer_id")
        intent.removeExtra("landa_focus_target")
        intent.removeExtra("landa_transfer_id")
        intent.removeExtra("landa_peer_id")

        val obj = JSObject().apply {
            put("target", target)
            if (transferId != null) put("transferId", transferId)
            if (peerId != null) put("peerId", peerId)
        }
        synchronized(lock) {
            pendingFocusTarget = obj
        }
        dispatchWebEvent("landa-focus-target")
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
