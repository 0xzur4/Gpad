package com.gpad.app

import android.hardware.usb.UsbAccessory
import android.hardware.usb.UsbManager
import android.os.Process
import android.os.SystemClock
import android.util.Log
import java.io.FileInputStream
import java.io.FileOutputStream
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger

/**
 * Tautan USB accessory ke daemon PC (Gpad v0.5.0).
 *
 * - openAccessory() -> stream bulk (TANPA ADB).
 * - Kirim handshake, terima balasan (info desktop & video).
 * - Heartbeat 1 dtk dua arah + echo; putus >3.5 dtk -> reconnect backoff.
 * - CH_VIDEO diteruskan ke VideoDecoder; CH_INPUT dikirim dari TouchView.
 */
class UsbLink(
    private val manager: UsbManager,
    private val onStatus: (String) -> Unit,
    private val onVideo: (videoPayload: ByteArray) -> Unit,
    private val onStats: (fps: Int, rttMs: Int) -> Unit,
    private val onReply: (Proto.HandshakeReply) -> Unit,
) {
    companion object {
        private const val TAG = "GpadUsb"
        private const val HB_INTERVAL_MS = 1000L
        private const val HB_DEAD_MS = 3500L
    }

    private val running = AtomicBoolean(false)
    private val seq = AtomicInteger(0)
    private val writeQueue = LinkedBlockingQueue<ByteArray>()
    private var worker: Thread? = null
    private val writeLock = Any()

    @Volatile private var out: FileOutputStream? = null

    val isConnected: Boolean get() = running.get()

    /** Mulai loop koneksi untuk accessory ini. */
    fun start(accessory: UsbAccessory, screenW: Int = 1080, screenH: Int = 2400) {
        if (running.getAndSet(true)) return
        worker = Thread({ loop(accessory, screenW, screenH) }, "gpad-usb").also { it.start() }
    }

    fun stop() {
        running.set(false)
        worker?.interrupt()
        try { out?.close() } catch (_: Exception) {}
        worker = null
    }

    /** Antrekan paket pena 24 byte (dipanggil dari UI thread). */
    fun sendPen(kind: Byte, x01: Float, y01: Float, flags: Byte = Proto.FLAG_FINGER) {
        if (!running.get()) return
        val pkt = Proto.penPacket(
            kind, seq.getAndIncrement(),
            (SystemClock.elapsedRealtimeNanos() / 1000L).toInt(),
            x01, y01, Proto.FINGER_PRESSURE, flags,
        )
        writeQueue.offer(Proto.frame(Proto.CH_INPUT, pkt))
    }

    private fun loop(accessory: UsbAccessory, screenW: Int, screenH: Int) {
        Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_DISPLAY)
        var attempt = 0
        while (running.get()) {
            try {
                onStatus("Menghubungkan ke PC…")
                val pfd = manager.openAccessory(accessory)
                    ?: throw Exception("openAccessory gagal (accessory mungkin terlepas)")
                val inp = FileInputStream(pfd.fileDescriptor)
                val outStream = FileOutputStream(pfd.fileDescriptor)
                synchronized(writeLock) { out = outStream }
                try {
                    session(inp, outStream, screenW, screenH)
                } finally {
                    synchronized(writeLock) { out = null }
                    try { inp.close() } catch (_: Exception) {}
                    try { outStream.close() } catch (_: Exception) {}
                    try { pfd.close() } catch (_: Exception) {}
                }
                attempt = 0 // sesi bersih -> reset backoff
                if (running.get()) {
                    onStatus("Terputus — mencoba lagi…")
                }
            } catch (e: InterruptedException) {
                break
            } catch (e: Exception) {
                Log.w(TAG, "sesi gagal: ${e.message}")
                onStatus("PC tidak terjangkau — cek kabel USB")
            }
            // Backoff: 1s, 2s, 4s, ... maks 30s.
            if (running.get()) {
                val delay = (1L shl attempt.coerceAtMost(5)).coerceAtMost(30L) * 1000L
                attempt++
                try { Thread.sleep(delay) } catch (_: InterruptedException) { break }
            }
        }
        onStatus("Terputus")
    }

    private fun session(inp: FileInputStream, outStream: FileOutputStream, screenW: Int, screenH: Int) {
        // --- tulis langsung (handshake butuh sebelum reader jalan) ---
        fun writeNow(channel: Int, payload: ByteArray) {
            val f = Proto.frame(channel, payload)
            synchronized(writeLock) {
                outStream.write(f)
                outStream.flush()
            }
        }

        // --- handshake: kirim info layar HP ---
        writeNow(Proto.CH_CONTROL, Proto.handshake(screenW.coerceAtLeast(1), screenH.coerceAtLeast(1)))
        Log.i(TAG, "handshake terkirim")

        // --- baca balasan handshake ---
        val (ch, payload) = readFrame(inp) ?: throw Exception("balasan handshake tak datang")
        if (ch != Proto.CH_CONTROL) throw Exception("balasan handshake: channel salah")
        val reply = Proto.parseReply(payload) ?: throw Exception("balasan handshake rusak")
        if (reply.version != Proto.VERSION) throw Exception("versi protokol tak cocok: ${reply.version}")
        Log.i(TAG, "handshake OK: desktop ${reply.deskW}x${reply.deskH}, video ${reply.vidW}x${reply.vidH}")
        onStatus("Tersambung — menggambar!")
        onReply(reply)

        // --- pompa tulis (antrean input) ---
        val writer = Thread {
            try {
                while (running.get()) {
                    val f = writeQueue.take()
                    synchronized(writeLock) {
                        outStream.write(f)
                        outStream.flush()
                    }
                }
            } catch (_: InterruptedException) {
            } catch (e: Exception) {
                Log.w(TAG, "writer mati: ${e.message}")
            }
        }.also { it.isDaemon = true; it.start() }

        try {
            // --- loop baca ---
            var lastHbSend = 0L
            var lastHbEcho = SystemClock.elapsedRealtime()
            var pendingHb: Long? = null
            while (running.get()) {
                val now = SystemClock.elapsedRealtime()
                // Heartbeat tiap detik.
                if (now - lastHbSend >= HB_INTERVAL_MS) {
                    lastHbSend = now
                    val ts = SystemClock.elapsedRealtimeNanos() / 1000L
                    pendingHb = ts
                    writeQueue.offer(Proto.frame(Proto.CH_HEARTBEAT, Proto.heartbeat(ts)))
                }
                // Mati bila echo tak kembali.
                if (pendingHb != null && now - lastHbEcho > HB_DEAD_MS) {
                    throw Exception("heartbeat tak di-echo >3.5 dtk")
                }
                val frame = readFrameTimeout(inp, 200) ?: continue
                lastHbEcho = now // ada trafik apa pun = hidup
                when (frame.first) {
                    Proto.CH_VIDEO -> onVideo(frame.second)
                    Proto.CH_HEARTBEAT -> {
                        val ts = Proto.parseHeartbeat(frame.second) ?: continue
                        if (pendingHb == ts) {
                            pendingHb = null
                        } else {
                            // Heartbeat dari PC — echo kembali.
                            writeQueue.offer(Proto.frame(Proto.CH_HEARTBEAT, Proto.heartbeat(ts)))
                        }
                    }
                    Proto.CH_STATS -> {
                        Proto.parseStats(frame.second)?.let { (fps, rtt, _) ->
                            onStats(fps, rtt)
                        }
                    }
                    else -> { /* abaikan */ }
                }
            }
        } finally {
            writer.interrupt()
        }
    }

    /** Baca satu frame lengkap (blocking). null bila stream tutup. */
    private fun readFrame(inp: FileInputStream): Pair<Int, ByteArray>? {
        val hdr = ByteArray(Proto.FRAME_HEADER_LEN)
        if (!readExact(inp, hdr, 0)) return null
        val (len, ch) = Proto.parseHeader(hdr) ?: throw Exception("header frame rusak")
        val payload = ByteArray(len - Proto.FRAME_HEADER_LEN)
        if (!readExact(inp, payload, 0)) return null
        return Pair(ch, payload)
    }

    /**
     * Baca satu frame dengan timeout total `timeoutMs`.
     * null bila timeout (bukan error) atau stream tutup.
     */
    private fun readFrameTimeout(inp: FileInputStream, timeoutMs: Long): Pair<Int, ByteArray>? {
        val deadline = SystemClock.elapsedRealtime() + timeoutMs
        val hdr = ByteArray(Proto.FRAME_HEADER_LEN)
        var off = 0
        while (off < hdr.size) {
            val remain = (deadline - SystemClock.elapsedRealtime()).toInt()
            if (remain <= 0) return null
            val avail = inp.available()
            if (avail <= 0) {
                Thread.sleep(5)
                continue
            }
            val n = inp.read(hdr, off, minOf(hdr.size - off, avail))
            if (n < 0) return null
            off += n
        }
        val (len, ch) = Proto.parseHeader(hdr) ?: throw Exception("header frame rusak")
        val payload = ByteArray(len - Proto.FRAME_HEADER_LEN)
        off = 0
        while (off < payload.size) {
            if (SystemClock.elapsedRealtime() >= deadline) {
                throw Exception("timeout baca payload")
            }
            val avail = inp.available()
            if (avail <= 0) {
                Thread.sleep(2)
                continue
            }
            val n = inp.read(payload, off, minOf(payload.size - off, avail))
            if (n < 0) throw Exception("stream tutup di tengah frame")
            off += n
        }
        return Pair(ch, payload)
    }

    private fun readExact(inp: FileInputStream, buf: ByteArray, timeoutMs: Long): Boolean {
        var off = 0
        while (off < buf.size) {
            val n = inp.read(buf, off, buf.size - off)
            if (n < 0) return false
            off += n
        }
        return true
    }
}
