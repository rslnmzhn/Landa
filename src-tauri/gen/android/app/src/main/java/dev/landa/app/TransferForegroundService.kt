package dev.landa.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat

class TransferForegroundService : Service() {
    private var wakeLock: PowerManager.WakeLock? = null
    private var wifiLock: WifiManager.WifiLock? = null

    companion object {
        const val ACTION_START_OR_UPDATE = "dev.landa.app.action.TRANSFER_FG_UPDATE"
        const val ACTION_STOP = "dev.landa.app.action.TRANSFER_FG_STOP"

        const val EXTRA_ACTIVE_COUNT = "active_count"
        const val EXTRA_DIRECTION = "direction"
        const val EXTRA_PEER_NAME = "peer_name"
        const val EXTRA_CURRENT_FILE = "current_file"
        const val EXTRA_FILES_DONE = "files_done"
        const val EXTRA_FILE_COUNT = "file_count"
        const val EXTRA_PERCENT = "percent"

        const val CHANNEL_PROGRESS = "landa_transfer_progress"
        const val FG_NOTIFICATION_ID = 19001
        private const val WAKE_LOCK_TIMEOUT_MS = 30L * 60L * 1000L

        fun ensureProgressChannel(context: Context) {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
            val nm = context.getSystemService(Context.NOTIFICATION_SERVICE) as? NotificationManager
                ?: return
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
        }

        fun buildTransferNotification(
            context: Context,
            activeCount: Int,
            direction: String,
            peerName: String,
            currentFile: String,
            filesDone: Int,
            fileCount: Int,
            percent: Int
        ): Notification {
            ensureProgressChannel(context)

            val tapIntent = Intent(context, MainActivity::class.java).apply {
                flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
                putExtra("landa_focus_target", "activity")
            }
            val pendingIntent = PendingIntent.getActivity(
                context,
                FG_NOTIFICATION_ID,
                tapIntent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
            )

            val fileName = currentFile.substringAfterLast('/').substringAfterLast('\\')
                .ifEmpty { peerName }
            val title = if (activeCount > 1) {
                "Landa · $peerName (+${activeCount - 1})"
            } else {
                "Landa · $peerName"
            }
            val contentText = if (fileCount > 1) {
                "$fileName (${(filesDone + 1).coerceAtMost(fileCount)}/$fileCount)"
            } else {
                fileName
            }
            val smallIcon = if (direction == "send") {
                android.R.drawable.stat_sys_upload
            } else {
                android.R.drawable.stat_sys_download
            }

            return NotificationCompat.Builder(context, CHANNEL_PROGRESS)
                .setSmallIcon(smallIcon)
                .setContentTitle(title)
                .setContentText(contentText)
                .setProgress(100, percent.coerceIn(0, 100), percent <= 0)
                .setOngoing(true)
                .setOnlyAlertOnce(true)
                .setContentIntent(pendingIntent)
                .setPriority(NotificationCompat.PRIORITY_LOW)
                .build()
        }
    }

    override fun onCreate() {
        super.onCreate()
        ensureProgressChannel(this)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            releaseLocks()
            ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
            stopSelf()
            return START_NOT_STICKY
        }

        acquireLocks()

        val activeCount = intent?.getIntExtra(EXTRA_ACTIVE_COUNT, 1) ?: 1
        val direction = intent?.getStringExtra(EXTRA_DIRECTION) ?: "receive"
        val peerName = intent?.getStringExtra(EXTRA_PEER_NAME) ?: "Landa"
        val currentFile = intent?.getStringExtra(EXTRA_CURRENT_FILE) ?: ""
        val filesDone = intent?.getIntExtra(EXTRA_FILES_DONE, 0) ?: 0
        val fileCount = intent?.getIntExtra(EXTRA_FILE_COUNT, 1) ?: 1
        val percent = intent?.getIntExtra(EXTRA_PERCENT, 0) ?: 0

        val notification = buildTransferNotification(
            context = this,
            activeCount = activeCount,
            direction = direction,
            peerName = peerName,
            currentFile = currentFile,
            filesDone = filesDone,
            fileCount = fileCount,
            percent = percent
        )

        val fgType = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
        } else {
            0
        }
        ServiceCompat.startForeground(this, FG_NOTIFICATION_ID, notification, fgType)
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        releaseLocks()
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun acquireLocks() {
        try {
            if (wakeLock?.isHeld != true) {
                val pm = applicationContext.getSystemService(Context.POWER_SERVICE) as? PowerManager
                wakeLock = pm?.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "landa:transfer_wake")?.apply {
                    setReferenceCounted(false)
                    acquire(WAKE_LOCK_TIMEOUT_MS)
                }
            }
        } catch (_: Exception) {
        }

        try {
            if (wifiLock?.isHeld != true) {
                val wm = applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager
                @Suppress("DEPRECATION")
                val mode = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                    WifiManager.WIFI_MODE_FULL_LOW_LATENCY
                } else {
                    WifiManager.WIFI_MODE_FULL_HIGH_PERF
                }
                wifiLock = wm?.createWifiLock(mode, "landa:transfer_wifi")?.apply {
                    setReferenceCounted(false)
                    acquire()
                }
            }
        } catch (_: Exception) {
        }
    }

    private fun releaseLocks() {
        try {
            if (wakeLock?.isHeld == true) {
                wakeLock?.release()
            }
        } catch (_: Exception) {
        } finally {
            wakeLock = null
        }

        try {
            if (wifiLock?.isHeld == true) {
                wifiLock?.release()
            }
        } catch (_: Exception) {
        } finally {
            wifiLock = null
        }
    }
}
