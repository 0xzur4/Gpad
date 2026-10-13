package com.gpad.app

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Path
import android.util.AttributeSet
import android.view.MotionEvent
import android.view.View
import kotlin.math.PI

/**
 * Kanvas sentuh Gpad (Fase 1).
 *
 * Menangkap stylus/jari lalu mengirim paket biner ke daemon PC:
 *  - ACTION_DOWN / MOVE / UP + titik historis (historySize)
 *  - ACTION_HOVER_MOVE untuk hover stylus
 *  - pressure, tilt, tombol stylus, tool type
 *  - palm rejection sederhana: jari diabaikan saat stylus aktif
 */
class PenView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
) : View(context, attrs) {

    var net: NetClient? = null

    private val inkPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        style = Paint.Style.STROKE
        strokeWidth = 6f
        color = 0xFF222222.toInt()
        strokeCap = Paint.Cap.ROUND
    }
    private val strokes = mutableListOf<Path>()
    private var current: Path? = null

    private var activePointerId = -1
    private var stylusDown = false

    init {
        isFocusable = true
        isFocusableInTouchMode = true
    }

    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
        super.onSizeChanged(w, h, oldw, oldh)
        net?.viewW = w
        net?.viewH = h
    }

    // --- hover stylus (tanpa sentuhan) ---
    override fun onHoverEvent(event: MotionEvent): Boolean {
        if (event.actionMasked == MotionEvent.ACTION_HOVER_MOVE &&
            event.getToolType(0) == MotionEvent.TOOL_TYPE_STYLUS
        ) {
            sendPoint(NetClient.KIND_HOVER, event, 0)
        }
        return true
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        // Minta event tanpa buffer agar jeda minimal (dampak di ACTION_DOWN dst).
        if (event.actionMasked == MotionEvent.ACTION_DOWN) {
            requestUnbufferedDispatch(event)
        }

        val net = net ?: return true
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN, MotionEvent.ACTION_POINTER_DOWN -> {
                val idx = event.actionIndex
                val tool = event.getToolType(idx)
                // Palm rejection: jari diabaikan selama stylus menyentuh.
                if (tool == MotionEvent.TOOL_TYPE_FINGER && stylusDown) return true
                activePointerId = event.getPointerId(idx)
                if (tool == MotionEvent.TOOL_TYPE_STYLUS) stylusDown = true
                current = Path().also {
                    it.moveTo(event.getX(idx), event.getY(idx))
                    strokes.add(it)
                }
                sendPoint(NetClient.KIND_DOWN, event, idx)
                invalidate()
            }
            MotionEvent.ACTION_MOVE -> {
                val idx = event.findPointerIndex(activePointerId)
                if (idx < 0) return true
                // Titik historis DULU (berurutan waktu), baru titik kini.
                // Mengabaikannya = garis patah-patah.
                for (h in 0 until event.historySize) {
                    sendHistorical(NetClient.KIND_MOVE, event, idx, h)
                    current?.lineTo(event.getHistoricalX(idx, h), event.getHistoricalY(idx, h))
                }
                sendPoint(NetClient.KIND_MOVE, event, idx)
                current?.lineTo(event.getX(idx), event.getY(idx))
                invalidate()
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_POINTER_UP -> {
                val idx = event.actionIndex
                if (event.getPointerId(idx) != activePointerId) return true
                for (h in 0 until event.historySize) {
                    sendHistorical(NetClient.KIND_MOVE, event, idx, h)
                }
                sendPoint(NetClient.KIND_UP, event, idx)
                if (event.getToolType(idx) == MotionEvent.TOOL_TYPE_STYLUS) stylusDown = false
                activePointerId = -1
                current = null
                invalidate()
            }
            MotionEvent.ACTION_CANCEL -> {
                activePointerId = -1
                stylusDown = false
                current = null
            }
        }
        return true
    }

    private fun flagsFor(event: MotionEvent, idx: Int): Byte {
        var f: Byte = 0
        val tool = event.getToolType(idx)
        if (tool == MotionEvent.TOOL_TYPE_FINGER) f = (f.toInt() or NetClient.FLAG_FINGER.toInt()).toByte()
        if (tool == MotionEvent.TOOL_TYPE_ERASER) f = (f.toInt() or NetClient.FLAG_ERASER.toInt()).toByte()
        if (event.buttonState and MotionEvent.BUTTON_STYLUS_PRIMARY != 0) {
            f = (f.toInt() or NetClient.FLAG_BARREL_BUTTON.toInt()).toByte()
        }
        return f
    }

    /** Pecah tilt+orientasi Android menjadi tilt_x/tilt_y derajat. */
    private fun tiltXY(event: MotionEvent, idx: Int): Pair<Int, Int> {
        val tiltRad = event.getAxisValue(MotionEvent.AXIS_TILT, idx) // 0=tegak .. PI/2=rebah
        val orient = event.getAxisValue(MotionEvent.AXIS_ORIENTATION, idx)
        val deg = Math.toDegrees(tiltRad.toDouble())
        val tx = (deg * kotlin.math.sin(orient)).toInt().coerceIn(-90, 90)
        val ty = (deg * kotlin.math.cos(orient)).toInt().coerceIn(-90, 90)
        return tx to ty
    }

    private fun sendPoint(kind: Byte, event: MotionEvent, idx: Int) {
        val (tx, ty) = tiltXY(event, idx)
        net?.sendPen(
            kind = kind,
            x01 = event.getX(idx) / width.coerceAtLeast(1),
            y01 = event.getY(idx) / height.coerceAtLeast(1),
            pressure01 = event.getPressure(idx),
            tiltX = tx,
            tiltY = ty,
            flags = flagsFor(event, idx),
        )
    }

    private fun sendHistorical(kind: Byte, event: MotionEvent, idx: Int, h: Int) {
        val (tx, ty) = tiltXY(event, idx)
        net?.sendPen(
            kind = kind,
            x01 = event.getHistoricalX(idx, h) / width.coerceAtLeast(1),
            y01 = event.getHistoricalY(idx, h) / height.coerceAtLeast(1),
            pressure01 = event.getHistoricalPressure(idx, h),
            tiltX = tx,
            tiltY = ty,
            flags = flagsFor(event, idx),
        )
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        canvas.drawColor(0xFFFAFAFA.toInt())
        for (p in strokes) canvas.drawPath(p, inkPaint)
    }

    fun clearInk() {
        strokes.clear()
        invalidate()
    }
}
