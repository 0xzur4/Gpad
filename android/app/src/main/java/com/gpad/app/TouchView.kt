package com.gpad.app

import android.content.Context
import android.util.AttributeSet
import android.view.MotionEvent
import android.view.View

/**
 * Lapisan sentuh transparan di atas video mirror.
 *
 * - Menghitung rect letterbox video dari aspek stream (balasan handshake).
 * - Sentuhan di dalam rect -> koordinat desktop 0..1 -> PenPacket ke PC.
 * - Sentuhan di area letterbox (bar hitam) diabaikan.
 * - Jari = FLAG_FINGER, tekanan emulasi konstan (Proto.FINGER_PRESSURE).
 */
class TouchView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
) : View(context, attrs) {

    var link: UsbLink? = null

    /** Aspek video dari handshake (vidW/vidH). 0 = belum tahu. */
    @Volatile var videoAspect: Float = 0f

    private var activePointerId = -1

    init {
        isFocusable = true
        isFocusableInTouchMode = true
    }

    /** Rect video (letterbox) dalam piksel view: [x, y, w, h]. */
    private fun videoRect(): FloatArray {
        val vw = width.toFloat()
        val vh = height.toFloat()
        val aspect = videoAspect
        if (vw <= 0 || vh <= 0 || aspect <= 0) return floatArrayOf(0f, 0f, 0f, 0f)
        // Letterbox: skala = min(vw/vidW, vh/vidH); dihitung via aspek.
        val w = minOf(vw, vh * aspect)
        val h = w / aspect
        return floatArrayOf((vw - w) / 2f, (vh - h) / 2f, w, h)
    }

    private fun mapToDesktop(tx: Float, ty: Float): Pair<Float, Float>? {
        val r = videoRect()
        val (rx, ry, rw, rh) = r
        if (rw <= 0 || rh <= 0) return null
        val nx = (tx - rx) / rw
        val ny = (ty - ry) / rh
        if (nx !in 0f..1f || ny !in 0f..1f) return null
        return Pair(nx, ny)
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        if (event.actionMasked == MotionEvent.ACTION_DOWN) {
            requestUnbufferedDispatch(event)
        }
        val link = link ?: return true
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN, MotionEvent.ACTION_POINTER_DOWN -> {
                val idx = event.actionIndex
                // Hanya satu jari aktif untuk menggambar.
                if (activePointerId != -1) return true
                activePointerId = event.getPointerId(idx)
                sendPoint(link, Proto.KIND_DOWN, event, idx)
            }
            MotionEvent.ACTION_MOVE -> {
                val idx = event.findPointerIndex(activePointerId)
                if (idx < 0) return true
                for (h in 0 until event.historySize) {
                    sendHistorical(link, event, idx, h)
                }
                sendPoint(link, Proto.KIND_MOVE, event, idx)
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_POINTER_UP -> {
                val idx = event.actionIndex
                if (event.getPointerId(idx) != activePointerId) return true
                for (h in 0 until event.historySize) {
                    sendHistorical(link, event, idx, h)
                }
                sendPoint(link, Proto.KIND_UP, event, idx)
                activePointerId = -1
            }
            MotionEvent.ACTION_CANCEL -> {
                activePointerId = -1
            }
        }
        return true
    }

    private fun sendPoint(link: UsbLink, kind: Byte, event: MotionEvent, idx: Int) {
        val (nx, ny) = mapToDesktop(event.getX(idx), event.getY(idx)) ?: return
        link.sendPen(kind, nx, ny)
    }

    private fun sendHistorical(link: UsbLink, event: MotionEvent, idx: Int, h: Int) {
        val (nx, ny) = mapToDesktop(
            event.getHistoricalX(idx, h),
            event.getHistoricalY(idx, h),
        ) ?: return
        link.sendPen(Proto.KIND_MOVE, nx, ny)
    }
}
