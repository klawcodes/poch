//! Backdrop kaca buram: tangkap layar di belakang notch, blur, simpan sebagai cache.

use super::*;

use std::cell::RefCell;

const DOWN: usize = 4;
const BLUR_R: usize = 2;
const PASSES: usize = 2;
const REFRESH: Duration = Duration::from_millis(140);
const SATURATE: f32 = 1.4;
const GRAIN: f32 = 0.0;

const _: () = assert!(W as usize % DOWN == 0 && H as usize % DOWN == 0);

struct Capture {
    dc: HDC,
    bits: *mut c_void,
    out: Vec<u8>,
    at: Option<Instant>,
    dead: bool,
}

thread_local! {
    static CAP: RefCell<Option<Capture>> = RefCell::new(None);
}

impl Capture {
    fn dead() -> Capture {
        Capture { dc: HDC::default(), bits: std::ptr::null_mut(), out: Vec::new(), at: None, dead: true }
    }
}

/// Window dikeluarkan dari capture supaya yang tertangkap hanya isi layar di belakangnya.
unsafe fn init(hwnd: HWND) -> Option<Capture> {
    SetWindowDisplayAffinity(hwnd, WINDOW_DISPLAY_AFFINITY(0x11)).ok()?;
    let screen = GetDC(None);
    let dc = CreateCompatibleDC(screen);
    ReleaseDC(None, screen);
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: W,
            biHeight: -H,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut c_void = std::ptr::null_mut();
    let bmp = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
    SelectObject(dc, bmp);
    Some(Capture { dc, bits, out: vec![0u8; (W * H * 4) as usize], at: None, dead: false })
}

fn box_blur(buf: &mut [f32], tmp: &mut [f32], w: usize, h: usize, r: usize) {
    let n = (2 * r + 1) as f32;
    for y in 0..h {
        for x in 0..w {
            for c in 0..3 {
                let mut s = 0.0;
                for k in 0..=2 * r {
                    s += buf[(y * w + (x + k).saturating_sub(r).min(w - 1)) * 3 + c];
                }
                tmp[(y * w + x) * 3 + c] = s / n;
            }
        }
    }
    for y in 0..h {
        for x in 0..w {
            for c in 0..3 {
                let mut s = 0.0;
                for k in 0..=2 * r {
                    s += tmp[((y + k).saturating_sub(r).min(h - 1) * w + x) * 3 + c];
                }
                buf[(y * w + x) * 3 + c] = s / n;
            }
        }
    }
}

/// Noise statis per piksel, jadi tidak berkedip saat backdrop di-refresh.
fn grain(i: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9E37_79B1);
    x ^= x >> 15;
    x = x.wrapping_mul(0x85EB_CA77);
    x ^= x >> 13;
    ((x & 0xff) as f32 / 255.0 - 0.5) * 2.0
}

unsafe fn grab(c: &mut Capture, wx: i32) -> bool {
    let screen = GetDC(None);
    let ok = BitBlt(c.dc, 0, 0, W, H, screen, wx, 0, ROP_CODE(SRCCOPY.0 | CAPTUREBLT.0)).is_ok();
    ReleaseDC(None, screen);
    if !ok {
        return false;
    }
    let (pw, ph) = (W as usize, H as usize);
    let src = std::slice::from_raw_parts(c.bits as *const u8, pw * ph * 4);
    let (sw, sh) = (pw / DOWN, ph / DOWN);

    let mut small = vec![0f32; sw * sh * 3];
    let norm = 1.0 / (DOWN * DOWN) as f32;
    for sy in 0..sh {
        for sx in 0..sw {
            let (mut r, mut g, mut b) = (0f32, 0f32, 0f32);
            for dy in 0..DOWN {
                let row = (sy * DOWN + dy) * pw + sx * DOWN;
                for dx in 0..DOWN {
                    let i = (row + dx) * 4;
                    b += src[i] as f32;
                    g += src[i + 1] as f32;
                    r += src[i + 2] as f32;
                }
            }
            let o = (sy * sw + sx) * 3;
            small[o] = r * norm;
            small[o + 1] = g * norm;
            small[o + 2] = b * norm;
        }
    }
    let mut tmp = vec![0f32; small.len()];
    for _ in 0..PASSES {
        box_blur(&mut small, &mut tmp, sw, sh, BLUR_R);
    }

    for y in 0..ph {
        let fy = ((y as f32 + 0.5) / DOWN as f32 - 0.5).clamp(0.0, (sh - 1) as f32);
        let (y0, ty) = (fy as usize, fy.fract());
        let y1 = (y0 + 1).min(sh - 1);
        for x in 0..pw {
            let fx = ((x as f32 + 0.5) / DOWN as f32 - 0.5).clamp(0.0, (sw - 1) as f32);
            let (x0, tx) = (fx as usize, fx.fract());
            let x1 = (x0 + 1).min(sw - 1);
            let mut px = [0f32; 3];
            for (ch, v) in px.iter_mut().enumerate() {
                let at = |yy: usize, xx: usize| small[(yy * sw + xx) * 3 + ch];
                let top = at(y0, x0) * (1.0 - tx) + at(y0, x1) * tx;
                let bot = at(y1, x0) * (1.0 - tx) + at(y1, x1) * tx;
                *v = top * (1.0 - ty) + bot * ty;
            }
            let lum = 0.299 * px[0] + 0.587 * px[1] + 0.114 * px[2];
            let n = grain((y * pw + x) as u32) * GRAIN;
            let o = (y * pw + x) * 4;
            for ch in 0..3 {
                c.out[o + ch] = (lum + (px[ch] - lum) * SATURATE + n).clamp(0.0, 255.0) as u8;
            }
            c.out[o + 3] = 255;
        }
    }
    true
}

/// Perbarui backdrop bila sudah basi. False = tidak tersedia, pakai kaca polos.
pub(crate) fn refresh(hwnd: isize, wx: i32) -> bool {
    CAP.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            let made = unsafe { init(HWND(hwnd as *mut c_void)) };
            *slot = Some(made.unwrap_or_else(Capture::dead));
        }
        let c = slot.as_mut().unwrap();
        if c.dead {
            return false;
        }
        if c.at.map_or(true, |t| t.elapsed() >= REFRESH) {
            if !unsafe { grab(c, wx) } {
                c.dead = true;
                return false;
            }
            c.at = Some(Instant::now());
        }
        true
    })
}

/// Salin backdrop ke pixmap hanya di dalam `bx`.
pub(crate) fn blit(pm: &mut Pixmap, bx: Box4) {
    CAP.with(|cell| {
        let slot = cell.borrow();
        let Some(c) = slot.as_ref() else { return; };
        if c.dead || c.out.is_empty() {
            return;
        }
        let pw = pm.width() as usize;
        let x0 = bx.0.min(pw);
        let x1 = bx.2.min(pw).max(x0);
        let y1 = bx.3.min(pm.height() as usize);
        let dst = pm.data_mut();
        for y in bx.1..y1 {
            let (lo, hi) = ((y * pw + x0) * 4, (y * pw + x1) * 4);
            dst[lo..hi].copy_from_slice(&c.out[lo..hi]);
        }
    });
}