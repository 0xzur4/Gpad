package com.gpad.app

import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * Konstanta protokol Gpad v0.5.0 — HARUS sama persis dengan gpad-protocol (Rust).
 *
 * Frame: [u32 LE panjang total][u8 channel][payload]
 */
object Proto {
    const val VERSION = 2
    const val MAGIC = 0x35445047 // "GPD5" little-endian

    // channel
    const val CH_CONTROL = 1
    const val CH_VIDEO = 2
    const val CH_INPUT = 3
    const val CH_HEARTBEAT = 4
    const val CH_STATS = 5

    // kind (PenPacket)
    const val KIND_DOWN: Byte = 0
    const val KIND_MOVE: Byte = 1
    const val KIND_UP: Byte = 2
    const val KIND_HOVER: Byte = 3

    // flags
    const val FLAG_FINGER: Byte = 0x04

    // caps
    const val CAP_PRESSURE = 0x0001
    const val CAP_HOVER = 0x0004
    const val CAP_VIDEO = 0x0010

    const val HANDSHAKE_LEN = 16
    const val HANDSHAKE_REPLY_LEN = 20
    const val PACKET_LEN = 24
    const val VIDEO_HEADER_LEN = 15
    const val FRAME_HEADER_LEN = 5
    const val MAX_FRAME_LEN = 8 * 1024 * 1024

    const val VIDEO_FLAG_KEYFRAME: Byte = 0x01

    /** Tekanan emulasi untuk jari (A56 tak punya stylus aktif). */
    const val FINGER_PRESSURE = 0.65f

    fun frame(channel: Int, payload: ByteArray): ByteArray {
        val total = FRAME_HEADER_LEN + payload.size
        require(total <= MAX_FRAME_LEN) { "frame terlalu besar" }
        val b = ByteBuffer.allocate(total).order(ByteOrder.LITTLE_ENDIAN)
        b.putInt(total)
        b.put(channel.toByte())
        b.put(payload)
        return b.array()
    }

    /** Parse header 5 byte -> (panjangTotal, channel) atau null bila invalid. */
    fun parseHeader(hdr: ByteArray): Pair<Int, Int>? {
        if (hdr.size < FRAME_HEADER_LEN) return null
        val len = ByteBuffer.wrap(hdr, 0, 4).order(ByteOrder.LITTLE_ENDIAN).int
        if (len < FRAME_HEADER_LEN || len > MAX_FRAME_LEN) return null
        return Pair(len, hdr[4].toInt() and 0xFF)
    }

    fun handshake(width: Int, height: Int): ByteArray {
        val b = ByteBuffer.allocate(HANDSHAKE_LEN).order(ByteOrder.LITTLE_ENDIAN)
        b.putInt(MAGIC)
        b.putShort(VERSION.toShort())
        b.putShort(width.toShort())
        b.putShort(height.toShort())
        b.putShort((CAP_PRESSURE or CAP_HOVER or CAP_VIDEO).toShort())
        b.putShort(0) // reserved
        return b.array()
    }

    /** Balasan PC -> HP (20 byte) -> Triple(deskW, deskH, fps) + video W/H. */
    data class HandshakeReply(
        val version: Int,
        val deskW: Int,
        val deskH: Int,
        val vidW: Int,
        val vidH: Int,
        val fps: Int,
    )

    fun parseReply(buf: ByteArray): HandshakeReply? {
        if (buf.size < HANDSHAKE_REPLY_LEN) return null
        val b = ByteBuffer.wrap(buf).order(ByteOrder.LITTLE_ENDIAN)
        if (b.int != MAGIC) return null
        return HandshakeReply(
            version = b.short.toInt() and 0xFFFF,
            deskW = b.short.toInt() and 0xFFFF,
            deskH = b.short.toInt() and 0xFFFF,
            vidW = b.short.toInt() and 0xFFFF,
            vidH = b.short.toInt() and 0xFFFF,
            fps = b.get().toInt() and 0xFF,
        )
    }

    /** Header video 15 byte -> Triple(seq, captureTsUs, flags). */
    fun parseVideoHeader(payload: ByteArray): Triple<Long, Long, Byte>? {
        if (payload.size < VIDEO_HEADER_LEN) return null
        val b = ByteBuffer.wrap(payload).order(ByteOrder.LITTLE_ENDIAN)
        val seq = b.int.toLong() and 0xFFFFFFFFL
        val ts = b.long
        val flags = b.get()
        return Triple(seq, ts, flags)
    }

    fun heartbeat(tsUs: Long): ByteArray =
        ByteBuffer.allocate(8).order(ByteOrder.LITTLE_ENDIAN).putLong(tsUs).array()

    fun parseHeartbeat(buf: ByteArray): Long? {
        if (buf.size < 8) return null
        return ByteBuffer.wrap(buf).order(ByteOrder.LITTLE_ENDIAN).long
    }

    /** Stats PC -> HP (5 byte) -> Triple(fps, rttMs, bitrateKbps). */
    fun parseStats(buf: ByteArray): Triple<Int, Int, Int>? {
        if (buf.size < 5) return null
        val b = ByteBuffer.wrap(buf).order(ByteOrder.LITTLE_ENDIAN)
        return Triple(
            b.get().toInt() and 0xFF,
            b.short.toInt() and 0xFFFF,
            b.short.toInt() and 0xFFFF,
        )
    }

    /** Encode PenPacket 24 byte little-endian. x/y: 0..1 relatif desktop PC. */
    fun penPacket(
        kind: Byte, seq: Int, tUs: Int,
        x: Float, y: Float, pressure: Float, flags: Byte,
    ): ByteArray {
        val b = ByteBuffer.allocate(PACKET_LEN).order(ByteOrder.LITTLE_ENDIAN)
        b.put(kind)
        b.put(flags)
        b.putShort(seq.toShort())
        b.putInt(tUs)
        b.putFloat(x.coerceIn(0f, 1f))
        b.putFloat(y.coerceIn(0f, 1f))
        b.putFloat(pressure.coerceIn(0f, 1f))
        b.putShort(0) // tilt_x
        b.putShort(0) // tilt_y
        return b.array()
    }

    /**
     * Pecah Annex-B menjadi daftar NAL (tanpa start code).
     * Mengembalikan list Pair(type, nalBytes).
     */
    fun splitNals(annexb: ByteArray): List<Pair<Int, ByteArray>> {
        val out = mutableListOf<Pair<Int, ByteArray>>()
        var i = 0
        var nalStart = -1
        fun flush(end: Int) {
            if (nalStart >= 0 && end > nalStart) {
                val nal = annexb.copyOfRange(nalStart, end)
                if (nal.isNotEmpty()) out.add(Pair(nal[0].toInt() and 0x1F, nal))
            }
            nalStart = -1
        }
        while (i < annexb.size) {
            if (annexb[i] == 0.toByte()) {
                var j = i
                while (j < annexb.size && annexb[j] == 0.toByte()) j++
                if (j < annexb.size && annexb[j] == 1.toByte() && j - i >= 2) {
                    flush(i)
                    nalStart = j + 1
                    i = j + 1
                    continue
                }
            }
            i++
        }
        flush(annexb.size)
        return out
    }
}
