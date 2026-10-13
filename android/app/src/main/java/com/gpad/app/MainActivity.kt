package com.gpad.app

import android.app.Activity
import android.graphics.Color
import android.os.Bundle
import android.view.Gravity
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.Button
import android.widget.FrameLayout
import android.widget.TextView

/**
 * Gpad Fase 1: kanvas sentuh fullscreen + indikator status koneksi.
 *
 * Cara pakai:
 *  1. Di PC (Windows): jalankan gpad-daemon.exe
 *  2. Colok HP via kabel USB DATA, aktifkan USB debugging
 *  3. Di PC: adb reverse tcp:51337 tcp:51337
 *     (atau biarkan daemon mencoba otomatis)
 *  4. Buka aplikasi ini — status berubah "Tersambung", kursor PC mengikuti jari/stylus
 */
class MainActivity : Activity() {

    private lateinit var net: NetClient
    private lateinit var statusView: TextView
    private lateinit var penView: PenView
    private lateinit var modeButton: Button
    private var wantRelative = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        @Suppress("DEPRECATION")
        window.decorView.systemUiVisibility =
            android.view.View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY or
            android.view.View.SYSTEM_UI_FLAG_FULLSCREEN or
            android.view.View.SYSTEM_UI_FLAG_HIDE_NAVIGATION

        net = NetClient(
            onStatus = { msg -> runOnUiThread { statusView.text = msg } },
            onMode = { mode ->
                runOnUiThread {
                    wantRelative = (mode == NetClient.MODE_RELATIVE)
                    refreshModeButton()
                }
            },
        )

        penView = PenView(this).also { it.net = net }

        statusView = TextView(this).apply {
            text = "Menghubungkan…"
            setTextColor(Color.WHITE)
            textSize = 14f
            gravity = Gravity.CENTER
            setBackgroundColor(Color.parseColor("#AA000000"))
            setPadding(24, 16, 24, 16)
        }

        modeButton = Button(this).apply {
            textSize = 14f
            setOnClickListener {
                wantRelative = !wantRelative
                net.sendModeSwitch(wantRelative)
                refreshModeButton()
            }
        }
        refreshModeButton()

        val root = FrameLayout(this).apply {
            addView(penView, FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            ))
            addView(statusView, FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
                Gravity.TOP or Gravity.CENTER_HORIZONTAL,
            ).apply { topMargin = 48 })
            addView(modeButton, FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
                Gravity.BOTTOM or Gravity.END,
            ).apply { bottomMargin = 48; rightMargin = 48 })
        }
        setContentView(root)
    }

    private fun refreshModeButton() {
        modeButton.text = if (wantRelative) "RELATIF" else "ABSOLUT"
    }

    override fun onResume() {
        super.onResume()
        net.start()
    }

    override fun onPause() {
        net.stop()
        super.onPause()
    }
}
