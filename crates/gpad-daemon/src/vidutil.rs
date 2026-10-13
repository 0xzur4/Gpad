//! vidutil.rs — utilitas video murni (tanpa cfg windows).
//!
//! - `fit_dims`: dimensi encode ≤1920×1080, genap, aspek dijaga.
//! - `scale_bgra_to_rgb_bilinear`: downscale BGRA -> RGB (buang alpha, tukar B/R).
//! - `bgr_to_yuv420`: konversi BT.601 integer ke planar YUV420.
//!
//! Semua fungsi bebas alokasi tersembunyi di jalur panas bila buffer
//! disediakan pemanggil; versi `vec` untuk kemudahan & pengujian.

/// Dimensi encode: muat dalam 1920×1080, jaga aspek, bulatkan ke genap
/// (syarat YUV420). Jika sumber sudah ≤1080p, kembalikan apa adanya
/// (tetap dibulatkan genap).
pub fn fit_dims(src_w: usize, src_h: usize) -> (usize, usize) {
    if src_w == 0 || src_h == 0 {
        return (0, 0);
    }
    let (mut w, mut h) = (src_w, src_h);
    if w > 1920 || h > 1080 {
        let s = (1920.0 / w as f32).min(1080.0 / h as f32);
        w = (w as f32 * s).round() as usize;
        h = (h as f32 * s).round() as usize;
    }
    // Genap ke bawah (H.264 YUV420 butuh dimensi genap).
    w &= !1;
    h &= !1;
    (w.max(2), h.max(2))
}

/// Bilinear downscale BGRA (src) -> RGB (dst). Buffer dst harus
/// dst_w*dst_h*3 byte. Hanya untuk downscale (faktor ≤1); untuk
/// upscale hasilnya tetap benar tapi tidak dioptimalkan.
pub fn scale_bgra_to_rgb_bilinear(
    src: &[u8],
    src_w: usize,
    src_h: usize,
    dst: &mut [u8],
    dst_w: usize,
    dst_h: usize,
) {
    assert_eq!(src.len(), src_w * src_h * 4);
    assert_eq!(dst.len(), dst_w * dst_h * 3);
    if src_w == dst_w && src_h == dst_h {
        // Jalur cepat: buang alpha + tukar B/R -> RGB.
        for (d, s) in dst.chunks_exact_mut(3).zip(src.chunks_exact(4)) {
            d[0] = s[2]; // R
            d[1] = s[1]; // G
            d[2] = s[0]; // B
        }
        return;
    }
    let x_ratio = src_w as f32 / dst_w as f32;
    let y_ratio = src_h as f32 / dst_h as f32;
    for dy in 0..dst_h {
        let sy = (dy as f32 + 0.5) * y_ratio - 0.5;
        let y0 = sy.floor().clamp(0.0, (src_h - 1) as f32) as usize;
        let y1 = (y0 + 1).min(src_h - 1);
        let fy = (sy - y0 as f32).clamp(0.0, 1.0);
        for dx in 0..dst_w {
            let sx = (dx as f32 + 0.5) * x_ratio - 0.5;
            let x0 = sx.floor().clamp(0.0, (src_w - 1) as f32) as usize;
            let x1 = (x0 + 1).min(src_w - 1);
            let fx = (sx - x0 as f32).clamp(0.0, 1.0);
            let di = (dy * dst_w + dx) * 3;
            // dst = RGB; src = BGRA -> kanal sumber = 2-c (R,G,B).
            for c in 0..3 {
                let sc = 2 - c;
                let p00 = src[(y0 * src_w + x0) * 4 + sc] as f32;
                let p10 = src[(y0 * src_w + x1) * 4 + sc] as f32;
                let p01 = src[(y1 * src_w + x0) * 4 + sc] as f32;
                let p11 = src[(y1 * src_w + x1) * 4 + sc] as f32;
                let v = p00 * (1.0 - fx) * (1.0 - fy)
                    + p10 * fx * (1.0 - fy)
                    + p01 * (1.0 - fx) * fy
                    + p11 * fx * fy;
                dst[di + c] = v.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// BGR -> YUV420 planar (BT.601, integer). `y` harus w*h byte,
/// `u`/`v` masing-masing (w/2)*(h/2) byte. w,h genap.
///
/// Catatan: jalur produksi memakai konverter openh264 (read_rgb8); fungsi
/// ini dipertahankan sebagai referensi teruji & fallback bebas-dependensi.
#[allow(dead_code)]
pub fn bgr_to_yuv420(bgr: &[u8], w: usize, h: usize, y: &mut [u8], u: &mut [u8], v: &mut [u8]) {
    assert_eq!(bgr.len(), w * h * 3);
    assert_eq!(y.len(), w * h);
    assert_eq!(u.len(), (w / 2) * (h / 2));
    assert_eq!(v.len(), (w / 2) * (h / 2));
    // Luma per piksel.
    for (i, px) in bgr.chunks_exact(3).enumerate() {
        let (b, g, r) = (px[0] as i32, px[1] as i32, px[2] as i32);
        y[i] = (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16).clamp(0, 255) as u8;
    }
    // Chroma: rata-rata blok 2x2.
    let hw = w / 2;
    for by in 0..h / 2 {
        for bx in 0..hw {
            let mut su = 0i32;
            let mut sv = 0i32;
            for dy in 0..2 {
                for dx in 0..2 {
                    let px = &bgr[(((by * 2 + dy) * w + (bx * 2 + dx)) * 3)..][..3];
                    let (b, g, r) = (px[0] as i32, px[1] as i32, px[2] as i32);
                    su += ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
                    sv += ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;
                }
            }
            let o = by * hw + bx;
            u[o] = (su / 4).clamp(0, 255) as u8;
            v[o] = (sv / 4).clamp(0, 255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_dims_keeps_small() {
        assert_eq!(fit_dims(1920, 1080), (1920, 1080));
        assert_eq!(fit_dims(1280, 720), (1280, 720));
    }

    #[test]
    fn fit_dims_downscales_1440p() {
        // 2560x1440 -> skala 0.75 -> 1920x1080.
        assert_eq!(fit_dims(2560, 1440), (1920, 1080));
    }

    #[test]
    fn fit_dims_4k_and_odd() {
        let (w, h) = fit_dims(3840, 2160);
        assert!(w <= 1920 && h <= 1080);
        assert_eq!(w % 2, 0);
        assert_eq!(h % 2, 0);
        // Aspek 16:9 terjaga.
        assert!(((w as f32 / h as f32) - 16.0 / 9.0).abs() < 0.01);
    }

    #[test]
    fn scale_identity_strips_alpha_and_swaps_to_rgb() {
        let src: Vec<u8> = (0..4 * 2 * 2).map(|i| i as u8).collect();
        let mut dst = vec![0u8; 3 * 2 * 2];
        scale_bgra_to_rgb_bilinear(&src, 2, 2, &mut dst, 2, 2);
        // BGRA [0,1,2,_] -> RGB [2,1,0].
        assert_eq!(&dst[0..3], &[2, 1, 0]);
        assert_eq!(&dst[9..12], &[14, 13, 12]);
    }

    #[test]
    fn scale_down_4x4_to_2x2_uniform() {
        // Abu-abu uniform -> hasil tetap abu-abu (bilinear eksak di uniform).
        let src = vec![128u8; 4 * 4 * 4];
        let mut dst = vec![0u8; 3 * 2 * 2];
        scale_bgra_to_rgb_bilinear(&src, 4, 4, &mut dst, 2, 2);
        assert!(dst.iter().all(|&b| b == 128));
    }

    #[test]
    fn yuv420_black_and_white() {
        let w = 4;
        let h = 4;
        // Hitam.
        let bgr = vec![0u8; w * h * 3];
        let mut y = vec![0u8; w * h];
        let mut u = vec![0u8; (w / 2) * (h / 2)];
        let mut v = vec![0u8; (w / 2) * (h / 2)];
        bgr_to_yuv420(&bgr, w, h, &mut y, &mut u, &mut v);
        assert!(y.iter().all(|&p| p == 16));
        assert!(u.iter().all(|&p| (p as i16 - 128).abs() <= 1));
        // Putih.
        let bgr = vec![255u8; w * h * 3];
        bgr_to_yuv420(&bgr, w, h, &mut y, &mut u, &mut v);
        assert!(y.iter().all(|&p| p == 235));
        assert!(u.iter().all(|&p| (p as i16 - 128).abs() <= 1));
    }

    #[test]
    fn yuv420_pure_red() {
        // Merah murni: Y ~ 82, V tinggi.
        let w = 2;
        let h = 2;
        let mut bgr = vec![0u8; w * h * 3];
        for px in bgr.chunks_exact_mut(3) {
            px[2] = 255;
        }
        let mut y = vec![0u8; w * h];
        let mut u = vec![0u8; 1];
        let mut v = vec![0u8; 1];
        bgr_to_yuv420(&bgr, w, h, &mut y, &mut u, &mut v);
        assert!((y[0] as i16 - 82).abs() <= 2, "y={}", y[0]);
        assert!(v[0] > 200, "v={}", v[0]);
    }
}
