package dev.landa.app

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.os.Bundle
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.widget.Button
import android.widget.FrameLayout
import android.widget.TextView
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat
import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.ChecksumException
import com.google.zxing.DecodeHintType
import com.google.zxing.FormatException
import com.google.zxing.NotFoundException
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

class QrScannerActivity : AppCompatActivity() {
    companion object {
        const val EXTRA_QR_URI = "qr_uri"
        const val EXTRA_ERROR = "qr_error"
    }

    private lateinit var previewView: PreviewView
    private lateinit var statusText: TextView
    private lateinit var cameraExecutor: ExecutorService
    private val qrReader = QRCodeReader()
    private val decodeHints = mapOf(
        DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE),
        DecodeHintType.TRY_HARDER to true
    )
    private val handled = AtomicBoolean(false)

    private val requestCameraPermission =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
            if (granted) {
                startCamera()
            } else {
                val data = Intent().putExtra(EXTRA_ERROR, "CAMERA_PERMISSION_DENIED")
                setResult(Activity.RESULT_CANCELED, data)
                finish()
            }
        }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        cameraExecutor = Executors.newSingleThreadExecutor()

        val root = FrameLayout(this).apply {
            setBackgroundColor(Color.BLACK)
        }

        previewView = PreviewView(this).apply {
            layoutParams = FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                FrameLayout.LayoutParams.MATCH_PARENT
            )
            scaleType = PreviewView.ScaleType.FILL_CENTER
        }
        root.addView(previewView)

        val frameSizePx = dp(240)
        val viewfinder = View(this).apply {
            layoutParams = FrameLayout.LayoutParams(frameSizePx, frameSizePx, Gravity.CENTER)
            background = GradientDrawable().apply {
                shape = GradientDrawable.RECTANGLE
                cornerRadius = dp(28).toFloat()
                setColor(Color.TRANSPARENT)
                setStroke(dp(3), Color.argb(220, 129, 140, 248))
            }
        }
        root.addView(viewfinder)

        statusText = TextView(this).apply {
            text = "Landa QR"
            setTextColor(Color.WHITE)
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 15f)
            gravity = Gravity.CENTER
            setPadding(dp(20), dp(10), dp(20), dp(10))
            background = GradientDrawable().apply {
                cornerRadius = dp(999).toFloat()
                setColor(Color.argb(150, 15, 23, 42))
                setStroke(dp(1), Color.argb(70, 255, 255, 255))
            }
            layoutParams = FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.WRAP_CONTENT,
                FrameLayout.LayoutParams.WRAP_CONTENT,
                Gravity.TOP or Gravity.CENTER_HORIZONTAL
            ).apply {
                topMargin = dp(56)
            }
        }
        root.addView(statusText)

        val cancelBtn = Button(this).apply {
            text = "✕"
            setTextColor(Color.WHITE)
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 18f)
            isAllCaps = false
            background = GradientDrawable().apply {
                cornerRadius = dp(999).toFloat()
                setColor(Color.argb(170, 15, 23, 42))
                setStroke(dp(1), Color.argb(90, 255, 255, 255))
            }
            layoutParams = FrameLayout.LayoutParams(dp(56), dp(56), Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL).apply {
                bottomMargin = dp(48)
            }
            setOnClickListener {
                setResult(Activity.RESULT_CANCELED)
                finish()
            }
        }
        root.addView(cancelBtn)

        setContentView(root)

        if (ContextCompat.checkSelfPermission(this, Manifest.permission.CAMERA)
            == PackageManager.PERMISSION_GRANTED
        ) {
            startCamera()
        } else {
            requestCameraPermission.launch(Manifest.permission.CAMERA)
        }
    }

    private fun startCamera() {
        val cameraProviderFuture = ProcessCameraProvider.getInstance(this)
        cameraProviderFuture.addListener({
            try {
                val cameraProvider = cameraProviderFuture.get()
                val preview = Preview.Builder().build().also {
                    it.surfaceProvider = previewView.surfaceProvider
                }

                val analysis = ImageAnalysis.Builder()
                    .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                    .build()
                    .also {
                        it.setAnalyzer(cameraExecutor) { imageProxy ->
                            analyzeFrame(imageProxy)
                        }
                    }

                cameraProvider.unbindAll()
                cameraProvider.bindToLifecycle(
                    this,
                    CameraSelector.DEFAULT_BACK_CAMERA,
                    preview,
                    analysis
                )
            } catch (ex: Exception) {
                val data = Intent().putExtra(EXTRA_ERROR, ex.message ?: "Camera unavailable")
                setResult(Activity.RESULT_CANCELED, data)
                finish()
            }
        }, ContextCompat.getMainExecutor(this))
    }

    private fun analyzeFrame(imageProxy: ImageProxy) {
        try {
            if (handled.get()) return
            val yPlane = imageProxy.planes.firstOrNull() ?: return
            val width = imageProxy.width
            val height = imageProxy.height
            val yBytes = extractYPlane(yPlane, width, height)
            val source = PlanarYUVLuminanceSource(yBytes, width, height, 0, 0, width, height, false)
            val bitmap = BinaryBitmap(HybridBinarizer(source))
            val result = try {
                qrReader.decode(bitmap, decodeHints)
            } catch (_: NotFoundException) {
                null
            } catch (_: ChecksumException) {
                null
            } catch (_: FormatException) {
                null
            } finally {
                qrReader.reset()
            } ?: return

            val raw = result.text?.trim() ?: return
            if (raw.startsWith("landa2://connect?")) {
                if (handled.compareAndSet(false, true)) {
                    runOnUiThread {
                        val data = Intent().putExtra(EXTRA_QR_URI, raw)
                        setResult(Activity.RESULT_OK, data)
                        finish()
                    }
                }
            } else if (raw.isNotEmpty()) {
                runOnUiThread {
                    statusText.text = "Not a Landa QR"
                }
            }
        } finally {
            imageProxy.close()
        }
    }

    private fun extractYPlane(plane: ImageProxy.PlaneProxy, width: Int, height: Int): ByteArray {
        val buffer = plane.buffer
        buffer.rewind()
        val rowStride = plane.rowStride
        val pixelStride = plane.pixelStride
        val out = ByteArray(width * height)
        if (pixelStride == 1 && rowStride == width) {
            buffer.get(out)
            return out
        }
        val rowBuffer = ByteArray(rowStride)
        for (row in 0 until height) {
            val remaining = buffer.remaining()
            val toRead = minOf(rowStride, remaining)
            if (toRead <= 0) break
            buffer.get(rowBuffer, 0, toRead)
            val dstOffset = row * width
            if (pixelStride == 1) {
                System.arraycopy(rowBuffer, 0, out, dstOffset, minOf(width, toRead))
            } else {
                for (col in 0 until width) {
                    val srcIdx = col * pixelStride
                    if (srcIdx < toRead) {
                        out[dstOffset + col] = rowBuffer[srcIdx]
                    }
                }
            }
        }
        return out
    }

    override fun onDestroy() {
        runCatching { cameraExecutor.shutdown() }
        super.onDestroy()
    }

    private fun dp(value: Int): Int {
        return TypedValue.applyDimension(
            TypedValue.COMPLEX_UNIT_DIP,
            value.toFloat(),
            resources.displayMetrics
        ).toInt()
    }
}
