package com.gpad.app

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.graphics.Color
import android.hardware.usb.UsbAccessory
import android.hardware.usb.UsbManager
import android.os.Bundle
import android.view.Gravity
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.Button
import android.widget.FrameLayout
import android.widget.TextView

/**
 * Gpad v0.5.0 "Cintiq mode".
 *
 * - HP jadi pen display: layar PC di-mirror via USB accessory (TANPA ADB).
 * - Satu tombol "Hubungkan"; status + fps tampil di atas.
 * - Gambar di HP sambil melihat layar HP; PC juga menampilkannya.
 *
 * Cara pakai:
 *  1. Di PC: jalankan gpad-daemon.exe (satu kali setup driver WinUSB
 *     via Zadig — lihat docs/driver-setup.md).
 *  2. Colok HP dengan kabel USB DATA.
 *  3. Buka aplikasi ini, tap "Hubungkan".
 */
class MainActivity : Activity() {

    private lateinit var usbManager: UsbManager
    private var link: UsbLink? = null
    private var decoder: VideoDecoder? = null

    private lateinit var statusView: TextView
    private lateinit var connectButton: Button
    private lateinit var surfaceView: SurfaceView
    private lateinit var touchView: TouchView

    private var pendingAccessory: UsbAccessory? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)

        usbManager = getSystemService(Context.USB_SERVICE) as UsbManager

        surfaceView = SurfaceView(this)
        touchView = TouchView(this)

        statusView = TextView(this).apply {
            text = "Colok USB, lalu tap Hubungkan"
            setTextColor(Color.WHITE)
            textSize = 14f
            gravity = Gravity.CENTER
            setBackgroundColor(Color.parseColor("#AA000000"))
            setPadding(24, 16, 24, 16)
        }

        connectButton = Button(this).apply {
            text = "Hubungkan"
            textSize = 18f
            setOnClickListener { onConnectButton() }
        }

        val root = FrameLayout(this).apply {
            setBackgroundColor(Color.BLACK)
            addView(surfaceView, FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            ))
            // Lapisan sentuh transparan di atas video.
            addView(touchView, FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            ))
            addView(statusView, FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
                Gravity.TOP or Gravity.CENTER_HORIZONTAL,
            ).apply { topMargin = 48 })
            addView(connectButton, FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
                Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL,
            ).apply { bottomMargin = 64 })
        }
        setContentView(root)

        surfaceView.holder.addCallback(object : SurfaceHolder.Callback {
            override fun surfaceCreated(holder: SurfaceHolder) {
                // Decoder dibuat saat koneksi (butuh surface yang valid).
            }
            override fun surfaceChanged(h: SurfaceHolder, f: Int, w: Int, h2: Int) {}
            override fun surfaceDestroyed(h: SurfaceHolder) {
                decoder?.stop()
                decoder = null
            }
        })

        // Dibuka via USB_ACCESSORY_ATTACHED?
        if (intent?.action == UsbManager.ACTION_USB_ACCESSORY_ATTACHED) {
            @Suppress("DEPRECATION")
            pendingAccessory = intent.getParcelableExtra(UsbManager.EXTRA_ACCESSORY)
            setStatus("Accessory terdeteksi — tap Hubungkan")
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        if (intent.action == UsbManager.ACTION_USB_ACCESSORY_ATTACHED) {
            @Suppress("DEPRECATION")
            pendingAccessory = intent.getParcelableExtra(UsbManager.EXTRA_ACCESSORY)
            setStatus("Accessory terdeteksi — tap Hubungkan")
        }
    }

    private fun onConnectButton() {
        if (link?.isConnected == true) {
            disconnect()
            return
        }
        val accessory = pendingAccessory
            ?: usbManager.accessoryList?.firstOrNull()
        if (accessory == null) {
            setStatus("Accessory tidak ditemukan — colok USB & jalankan daemon dulu")
            return
        }
        connect(accessory)
    }

    private fun connect(accessory: UsbAccessory) {
        setStatus("Menghubungkan…")
        connectButton.isEnabled = false

        val surface = surfaceView.holder.surface
        val dec = VideoDecoder(surface) { err ->
            runOnUiThread { setStatus(err) }
        }
        decoder = dec

        val newLink = UsbLink(
            manager = usbManager,
            onStatus = { msg -> runOnUiThread { setStatus(msg) } },
            onVideo = { payload -> dec.feed(payload) },
            onStats = { fps, rtt -> runOnUiThread { setStatus("Tersambung — $fps fps, RTT ${rtt}ms") } },
            onReply = { reply ->
                runOnUiThread {
                    touchView.videoAspect =
                        if (reply.vidH > 0) reply.vidW.toFloat() / reply.vidH else 0f
                    connectButton.text = "Putuskan"
                    connectButton.isEnabled = true
                }
            },
        )
        link = newLink
        dec.start()
        val dm = resources.displayMetrics
        newLink.start(accessory, dm.widthPixels, dm.heightPixels)
    }

    private fun disconnect() {
        link?.stop()
        link = null
        decoder?.stop()
        decoder = null
        connectButton.text = "Hubungkan"
        connectButton.isEnabled = true
        setStatus("Terputus")
    }

    private fun setStatus(msg: String) {
        statusView.text = msg
    }

    override fun onPause() {
        // Jangan putus saat screen-off sesaat? v0.5.0: putus agar hemat.
        // (Pengguna tap Hubungkan lagi.)
        super.onPause()
    }

    override fun onDestroy() {
        disconnect()
        super.onDestroy()
    }
}
