package com.gpad.app

import android.media.MediaCodec
import android.media.MediaFormat
import android.os.SystemClock
import android.util.Log
import android.view.Surface
import java.nio.ByteBuffer
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Decoder H.264 (MediaCodec hardware) -> Surface.
 *
 * - Menunggu keyframe pertama berisi SPS/PPS -> konfigurasi csd-0/csd-1.
 * - Tiap payload video (satu access unit Annex-B) jadi satu input buffer.
 * - Bila codec error, lapor via onError agar sesi bisa restart decoder.
 */
class VideoDecoder(
    private val surface: Surface,
    private val onError: (String) -> Unit,
) {
    companion object {
        private const val TAG = "GpadVideo"
        private const val MIME = "video/avc"
    }

    private val queue = LinkedBlockingQueue<ByteArray>()
    private val running = AtomicBoolean(false)
    private var worker: Thread? = null

    @Volatile var framesDecoded: Long = 0
        private set

    fun start() {
        if (running.getAndSet(true)) return
        worker = Thread(::loop, "gpad-decode").also { it.start() }
    }

    fun stop() {
        running.set(false)
        worker?.interrupt()
        worker = null
        queue.clear()
    }

    /** Antrekan payload video (VideoHeader 15 byte + Annex-B). */
    fun feed(payload: ByteArray) {
        if (!running.get()) return
        // Jangan antre: decoder tertinggal -> buang frame lama.
        if (queue.size > 3) queue.poll()
        queue.offer(payload)
    }

    private fun loop() {
        var codec: MediaCodec? = null
        try {
            // 1. Tunggu keyframe dengan SPS/PPS.
            var sps: ByteArray? = null
            var pps: ByteArray? = null
            var vidW = 0
            var vidH = 0
            var firstPayload: ByteArray? = null
            while (running.get() && sps == null) {
                val payload = queue.take()
                val vh = Proto.parseVideoHeader(payload) ?: continue
                if (vh.third.toInt() and Proto.VIDEO_FLAG_KEYFRAME.toInt() == 0) continue
                val annexb = payload.copyOfRange(Proto.VIDEO_HEADER_LEN, payload.size)
                for ((type, nal) in Proto.splitNals(annexb)) {
                    when (type) {
                        7 -> if (sps == null) sps = nal
                        8 -> if (pps == null) pps = nal
                    }
                }
                if (sps != null && pps != null) {
                    firstPayload = payload
                    // Dimensi dari SPS (opsional; pakai reply handshake bila gagal).
                    parseSpsDimensions(sps!!)?.let { (w, h) -> vidW = w; vidH = h }
                }
            }
            if (!running.get()) return
            Log.i(TAG, "SPS/PPS dapat, konfigurasi decoder")

            // 2. Konfigurasi codec.
            codec = MediaCodec.createDecoderByType(MIME)
            val format = MediaFormat.createVideoFormat(MIME, vidW.coerceAtLeast(16), vidH.coerceAtLeast(16))
            format.setByteBuffer("csd-0", ByteBuffer.wrap(sps!!))
            format.setByteBuffer("csd-1", ByteBuffer.wrap(pps!!))
            codec.configure(format, surface, null, 0)
            codec.start()

            // 3. Umpankan keyframe pertama + loop.
            firstPayload?.let { feedFrame(codec!!, it) }
            val info = MediaCodec.BufferInfo()
            var ptsUs = 0L
            while (running.get()) {
                // Input.
                val inIdx = codec.dequeueInputBuffer(10_000)
                if (inIdx >= 0) {
                    val payload = queue.poll()
                    if (payload == null) {
                        // Tak ada frame baru: kembalikan buffer kosong.
                        codec.queueInputBuffer(inIdx, 0, 0, ptsUs, 0)
                    } else {
                        val annexb = payload.copyOfRange(Proto.VIDEO_HEADER_LEN, payload.size)
                        val buf = codec.getInputBuffer(inIdx)!!
                        if (buf.remaining() < annexb.size) {
                            Log.w(TAG, "input buffer kekecilan (${buf.remaining()} < ${annexb.size})")
                            codec.queueInputBuffer(inIdx, 0, 0, ptsUs, 0)
                        } else {
                            buf.clear()
                            buf.put(annexb)
                            ptsUs += 33_333
                            codec.queueInputBuffer(inIdx, 0, annexb.size, ptsUs, 0)
                        }
                    }
                }
                // Output -> surface.
                val outIdx = codec.dequeueOutputBuffer(info, 10_000)
                when {
                    outIdx >= 0 -> {
                        codec.releaseOutputBuffer(outIdx, true)
                        framesDecoded++
                    }
                    outIdx == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> {
                        val f = codec.outputFormat
                        Log.i(TAG, "format output: ${f.getInteger("width")}x${f.getInteger("height")}")
                    }
                }
            }
        } catch (e: InterruptedException) {
            // stop() normal
        } catch (e: Exception) {
            Log.e(TAG, "decoder error: ${e.message}")
            onError("Decoder error: ${e.message}")
        } finally {
            try { codec?.stop() } catch (_: Exception) {}
            try { codec?.release() } catch (_: Exception) {}
        }
    }

    private fun feedFrame(codec: MediaCodec, payload: ByteArray) {
        val annexb = payload.copyOfRange(Proto.VIDEO_HEADER_LEN, payload.size)
        val inIdx = codec.dequeueInputBuffer(100_000)
        if (inIdx >= 0) {
            val buf = codec.getInputBuffer(inIdx)!!
            buf.clear()
            if (buf.remaining() >= annexb.size) {
                buf.put(annexb)
                codec.queueInputBuffer(inIdx, 0, annexb.size, SystemClock.elapsedRealtimeNanos() / 1000, 0)
            }
        }
    }

    /**
     * Parse dimensi dari SPS (baseline). Mengembalikan null bila gagal —
     * pemanggil memakai dimensi dari handshake sebagai fallback.
     */
    private fun parseSpsDimensions(sps: ByteArray): Pair<Int, Int>? {
        return try {
            val br = BitReader(sps)
            br.readBits(8) // NAL header
            br.readBits(8) // profile_idc
            br.readBits(8) // constraint flags
            br.readBits(8) // level_idc
            br.readUE() // seq_parameter_set_id
            val profile = sps[1].toInt() and 0xFF
            if (profile == 100 || profile == 110 || profile == 122 || profile == 244 ||
                profile == 44 || profile == 83 || profile == 86 || profile == 118 ||
                profile == 128 || profile == 138 || profile == 139 || profile == 134
            ) {
                val chroma = br.readUE()
                if (chroma == 3) br.readBits(1)
                br.readUE(); br.readUE()
                br.readBits(1)
                if (br.readBits(1) == 1) {
                    for (i in 0 until 8) {
                        if (br.readBits(1) == 1) {
                            val lastScale = 8
                            val size = if (i < 6) 16 else 64
                            var nextScale = 8
                            for (j in 0 until size) {
                                if (nextScale != 0) {
                                    val delta = br.readSE()
                                    nextScale = (lastScale + delta + 256) % 256
                                }
                            }
                        }
                    }
                }
            }
            br.readUE() // log2_max_frame_num_minus4
            val pocType = br.readUE()
            if (pocType == 0) br.readUE() else if (pocType == 1) {
                br.readBits(1); br.readSE(); br.readSE()
                val n = br.readUE()
                for (i in 0 until n) br.readSE()
            }
            br.readUE() // max_num_ref_frames
            br.readBits(1) // gaps_in_frame_num
            val picW = br.readUE()
            val picH = br.readUE()
            val frameMbsOnly = br.readBits(1)
            if (frameMbsOnly == 0) br.readBits(1)
            br.readBits(1) // direct_8x8_inference
            val crop = br.readBits(1)
            var cropL = 0; var cropR = 0; var cropT = 0; var cropB = 0
            if (crop == 1) {
                cropL = br.readUE(); cropR = br.readUE()
                cropT = br.readUE(); cropB = br.readUE()
            }
            val w = (picW + 1) * 16 - (cropL + cropR) * 2
            val h = (picH + 1) * 16 * (2 - frameMbsOnly) - (cropT + cropB) * 2
            Pair(w, h)
        } catch (_: Exception) {
            null
        }
    }

    /** Bit reader dengan Emulation Prevention (0x000003 -> 0x0000). */
    private class BitReader(sps: ByteArray) {
        private val data: ByteArray
        private var bytePos = 0
        private var bitPos = 0

        init {
            // Buang emulation prevention bytes.
            val out = mutableListOf<Byte>()
            var i = 0
            while (i < sps.size) {
                if (i + 2 < sps.size && sps[i] == 0.toByte() && sps[i + 1] == 0.toByte() && sps[i + 2] == 3.toByte()) {
                    out.add(0); out.add(0)
                    i += 3
                } else {
                    out.add(sps[i]); i++
                }
            }
            data = out.toByteArray()
        }

        fun readBits(n: Int): Int {
            var v = 0
            repeat(n) {
                v = v shl 1
                if (bytePos < data.size) {
                    val b = data[bytePos].toInt() and 0xFF
                    v = v or ((b shr (7 - bitPos)) and 1)
                    bitPos++
                    if (bitPos == 8) { bitPos = 0; bytePos++ }
                }
            }
            return v
        }

        fun readUE(): Int {
            var zeros = 0
            while (readBits(1) == 0 && zeros < 32) zeros++
            return (1 shl zeros) - 1 + readBits(zeros)
        }

        fun readSE(): Int {
            val ue = readUE()
            return if (ue % 2 == 0) -(ue / 2) else (ue + 1) / 2
        }
    }
}
