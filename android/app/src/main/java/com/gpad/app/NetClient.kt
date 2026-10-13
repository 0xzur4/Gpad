package com.gpad.app

import android.os.Process
import android.os.SystemClock
import android.util.Log
import java.io.DataOutputStream
import java.net.Socket
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.ArrayDeque
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger

/**
 * Klien TCP ke daemon Gpad di PC.
 * Aplikasi Android terhubung ke 127.0.0.1:51337 — ADB reverse
 * (adb reverse tcp:51337 tcp:51337) meneruskannya lewat kabel USB ke PC.
 *
 * Protokol (harus sama persis dengan gpad-protocol di Rust):
 *  - Handshake 16 byte: magic "GPAD" | version u16 | caps u16 |
 *    width u16 | height u16 | reserved u32  (little-endian)
 *  - PenPacket 24 byte per event (little-endian)
 */
class NetClient(
    private val onStatus: (String) -> Unit,
    private val onMode: (Int) -> Unit = {},
) {
    companion object {
        private const val TAG = "GpadNet"
        private const val HOST = "127.0.0.1"
        private const val PORT = 51337

        // kind
        const val KIND_DOWN: Byte = 0
        const val KIND_MOVE: Byte = 1
        const val KIND_UP: Byte = 2
        const val KIND_HOVER: Byte = 3
        const val KIND_MODE_SWITCH: Byte = 4

        // flags
        const val FLAG_BARREL_BUTTON: Byte = 0x01
        const val FLAG_ERASER: Byte = 0x02
        const val FLAG_FINGER: Byte = 0x04

        // caps
        const val CAP_PRESSURE: Int = 0x0001
        const val CAP_TILT: Int = 0x0002
        const val CAP_HOVER: Int = 0x0004
        const val CAP_BUTTON: Int = 0x0008

        // mode (sama dengan gpad-protocol)
        const val MODE_ABSOLUTE: Int = 0
        const val MODE_RELATIVE: Int = 1
        private const val MODE_REPLY_MAGIC: Int = 0x444F4D47
    }

    private val queue = LinkedBlockingQueue<ByteArray>()
    private val running = AtomicBoolean(false)
    private val seq = AtomicInteger(0)
    private var worker: Thread? = null

    // Fase 4: pool array 24-byte — jalur panas (setiap titik sentuh) TANPA
    // alokasi, tanpa tekanan GC. Produsen ambil dari pool, konsumen kembalikan.
    private val pool = ArrayDeque<ByteArray>(128)

    private fun takeBuf(): ByteArray = synchronized(pool) {
        pool.pollFirst() ?: ByteArray(24)
    }

    private fun giveBuf(b: ByteArray) = synchronized(pool) {
        if (pool.size < 128) pool.addLast(b)
    }

    /** Lebar/tinggi kanvas piksel — dikirim saat handshake. */
    @Volatile var viewW: Int = 0
    @Volatile var viewH: Int = 0

    val isConnected: Boolean get() = running.get()

    fun start() {
        if (running.getAndSet(true)) return
        worker = Thread(::loop, "gpad-net").also { it.start() }
    }

    fun stop() {
        running.set(false)
        worker?.interrupt()
        worker = null
    }

    /** Antrekan paket pena 24 byte. Dipanggil dari UI thread; pengiriman di worker.
     *  Fase 4: tanpa alokasi — pakai pool + encode manual little-endian. */
    fun sendPen(
        kind: Byte,
        x01: Float,          // 0.0..1.0 relatif ke kanvas
        y01: Float,
        pressure01: Float,
        tiltX: Int,
        tiltY: Int,
        flags: Byte,
    ) {
        if (!running.get()) return
        queue.offer(encodePacket(kind, flags.toInt(), x01, y01, pressure01, tiltX, tiltY))
    }

    /** Minta daemon ganti mode: kind=4, bit0 flags = 0 absolut / 1 relatif. */
    fun sendModeSwitch(relative: Boolean) {
        if (!running.get()) return
        queue.offer(encodePacket(KIND_MODE_SWITCH, if (relative) 1 else 0, 0f, 0f, 0f, 0, 0))
    }

    /** Encode PenPacket 24 byte little-endian TANPA ByteBuffer (tanpa alokasi). */
    private fun encodePacket(
        kind: Byte, flags: Int, x: Float, y: Float,
        pressure: Float, tiltX: Int, tiltY: Int,
    ): ByteArray {
        val b = takeBuf()
        b[0] = kind
        b[1] = flags.toByte()
        val s = seq.getAndIncrement()
        b[2] = (s and 0xFF).toByte()
        b[3] = ((s ushr 8) and 0xFF).toByte()
        putIntLE(b, 4, (SystemClock.elapsedRealtimeNanos() / 1000L).toInt())
        putFloatLE(b, 8, x.coerceIn(0f, 1f))
        putFloatLE(b, 12, y.coerceIn(0f, 1f))
        putFloatLE(b, 16, pressure.coerceIn(0f, 1f))
        val tx = tiltX.coerceIn(-90, 90)
        val ty = tiltY.coerceIn(-90, 90)
        b[20] = (tx and 0xFF).toByte()
        b[21] = ((tx ushr 8) and 0xFF).toByte()
        b[22] = (ty and 0xFF).toByte()
        b[23] = ((ty ushr 8) and 0xFF).toByte()
        return b
    }

    private fun putIntLE(b: ByteArray, off: Int, v: Int) {
        b[off] = (v and 0xFF).toByte()
        b[off + 1] = ((v ushr 8) and 0xFF).toByte()
        b[off + 2] = ((v ushr 16) and 0xFF).toByte()
        b[off + 3] = ((v ushr 24) and 0xFF).toByte()
    }

    private fun putFloatLE(b: ByteArray, off: Int, v: Float) {
        putIntLE(b, off, java.lang.Float.floatToIntBits(v))
    }

    private fun loop() {
        // Fase 4: prioritas tinggi agar titik sentuh tidak antre di belakang
        // pekerjaan UI/render lain.
        Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_DISPLAY)
        while (running.get()) {
            try {
                onStatus("Menghubungkan ke PC…")
                Socket(HOST, PORT).use { sock ->
                    sock.tcpNoDelay = true   // paket kecil jangan ditunda Nagle
                    val out = DataOutputStream(sock.getOutputStream())
                    val inp = sock.getInputStream()

                    // --- handshake 16 byte ---
                    val hb = ByteBuffer.allocate(16).order(ByteOrder.LITTLE_ENDIAN)
                    hb.putInt(0x44415047)          // "GPAD" little-endian
                    hb.putShort(1)                 // versi protokol
                    hb.putShort((CAP_PRESSURE or CAP_TILT or CAP_HOVER or CAP_BUTTON).toShort())
                    hb.putShort(viewW.coerceAtLeast(1).toShort())
                    hb.putShort(viewH.coerceAtLeast(1).toShort())
                    hb.putInt(0)                   // reserved
                    out.write(hb.array())
                    out.flush()

                    onStatus("Tersambung — gambar!")
                    Log.i(TAG, "connected, handshake sent")

                    // --- thread pembaca balasan mode (8 byte "GMOD") ---
                    val reader = Thread {
                        val rb = ByteArray(8)
                        try {
                            while (running.get() && !sock.isClosed) {
                                var off = 0
                                while (off < 8) {
                                    val n = inp.read(rb, off, 8 - off)
                                    if (n < 0) return@Thread
                                    off += n
                                }
                                val bb = ByteBuffer.wrap(rb).order(ByteOrder.LITTLE_ENDIAN)
                                if (bb.int == MODE_REPLY_MAGIC) {
                                    val mode = bb.get().toInt()
                                    if (mode == MODE_ABSOLUTE || mode == MODE_RELATIVE) {
                                        onMode(mode)
                                    }
                                }
                            }
                        } catch (_: Exception) { /* socket tutup */ }
                    }.also { it.isDaemon = true; it.start() }

                    // --- pompa antrean (Fase 4: batch + kembalikan buffer ke pool) ---
                    val batch = ArrayList<ByteArray>(16)
                    try {
                        while (running.get() && !sock.isClosed) {
                            batch.clear()
                            batch.add(queue.take())          // blokir sampai ada 1 paket
                            queue.drainTo(batch, 15)         // sapu sisanya tanpa blokir
                            for (pkt in batch) {
                                out.write(pkt)
                                giveBuf(pkt)                 // daur ulang, tanpa GC
                            }
                            out.flush()                      // 1 flush per batch
                        }
                    } finally {
                        reader.interrupt()
                    }
                }
            } catch (e: InterruptedException) {
                break
            } catch (e: Exception) {
                Log.w(TAG, "koneksi gagal: ${e.message}")
                onStatus("PC tidak terjangkau — cek kabel + adb reverse")
                try { Thread.sleep(2000) } catch (_: InterruptedException) { break }
            }
        }
        onStatus("Terputus")
    }
}
