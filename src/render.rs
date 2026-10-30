//! Menggambar satu frame (notch kecil + slide) dan menampilkannya ke window.

use super::*;

use crate::config::{GLASS_ALPHA, GLASS_BLUR};
use std::cell::RefCell;
use windows::Win32::Graphics::Dwm::{DwmEnableBlurBehindWindow, DWM_BB_BLURREGION, DWM_BB_ENABLE, DWM_BLURBEHIND};
use windows::Win32::Graphics::Gdi::{CombineRgn, CreateRectRgn, DeleteObject, HGDIOBJ, RGN_OR};

type Rect4 = (i32, i32, i32, i32);

thread_local! {
    static BLUR_RECTS: RefCell<Vec<Rect4>> = RefCell::new(Vec::new());
}

/// Rects covering only the pixels the notch fully covers. The blur edge is not anti-aliased,
/// so it stays 1px inside the shape and hides under the tinted, anti-aliased body edge.
fn blur_rects(g: &Gfx) -> Vec<Rect4> {
    let Some(mask) = g.mask.as_ref() else { return Vec::new(); };
    let data = mask.data();
    let pw = g.pm.width() as usize;
    let (x0, y0, x1, y1) = (g.prev_box.0.min(pw), g.prev_box.1, g.prev_box.2.min(pw), g.prev_box.3.min(g.pm.height() as usize));
    let mut out: Vec<Rect4> = Vec::new();
    for y in y0..y1 {
        let row = &data[y * pw + x0..y * pw + x1];
        let Some(first) = row.iter().position(|&c| c >= 250) else { continue; };
        let last = row.iter().rposition(|&c| c >= 250).unwrap_or(first);
        let (l, r) = ((x0 + first) as i32, (x0 + last + 1) as i32);
        match out.last_mut() {
            Some(q) if q.0 == l && q.2 == r && q.3 == y as i32 => q.3 += 1,
            _ => out.push((l, y as i32, r, y as i32 + 1)),
        }
    }
    out
}

/// Blur-behind only the notch region. Skipped when the region did not change since the last call.
unsafe fn apply_blur(hwnd: HWND, rects: Vec<Rect4>) {
    let same = BLUR_RECTS.with(|c| *c.borrow() == rects);
    if same {
        return;
    }
    let rgn = CreateRectRgn(0, 0, 0, 0);
    for &(x0, y0, x1, y1) in &rects {
        let part = CreateRectRgn(x0, y0, x1, y1);
        CombineRgn(rgn, rgn, part, RGN_OR);
        let _ = DeleteObject(HGDIOBJ(part.0));
    }
    let bb = DWM_BLURBEHIND {
        dwFlags: DWM_BB_ENABLE | DWM_BB_BLURREGION,
        fEnable: true.into(),
        hRgnBlur: rgn,
        fTransitionOnMaximized: false.into(),
    };
    let _ = DwmEnableBlurBehindWindow(hwnd, &bb);
    let _ = DeleteObject(HGDIOBJ(rgn.0));
    BLUR_RECTS.with(|c| *c.borrow_mut() = rects);
}

/// Apply the notch-shape mask, but only over `bx` (cheap even at 60 fps).
fn apply_mask(g: &mut Gfx, shape: &Path, bx: Box4) {
    let Some(mut mask) = g.mask.take() else { return; };
    mask.data_mut().fill(0);
    mask.fill_path(shape, FillRule::Winding, true, Transform::identity());
    let pm_w = g.pm.width() as usize;
    let pm_h = g.pm.height() as usize;
    let x0 = bx.0.min(pm_w);
    let y0 = bx.1.min(pm_h);
    let x1 = bx.2.min(pm_w).max(x0);
    let y1 = bx.3.min(pm_h).max(y0);
    let mask_data = mask.data();
    let pm_data = g.pm.data_mut();
    for y in y0..y1 {
        let row = y * pm_w;
        for x in x0..x1 {
            let idx = row + x;
            let c = mask_data[idx];
            if c == 255 {
                continue;
            }
            let cf = c as u32;
            let base = idx * 4;
            pm_data[base] = ((pm_data[base] as u32 * cf + 127) / 255) as u8;
            pm_data[base + 1] = ((pm_data[base + 1] as u32 * cf + 127) / 255) as u8;
            pm_data[base + 2] = ((pm_data[base + 2] as u32 * cf + 127) / 255) as u8;
            pm_data[base + 3] = ((pm_data[base + 3] as u32 * cf + 127) / 255) as u8;
        }
    }
    g.mask = Some(mask);
}

/// Notch yang seluruh bentuknya bergelombang setelah ada item di-drop (t = detik sejak drop).
/// - Tepi bawah melorot (sag) lalu memantul, paling kuat di tengah.
/// - Riak merambat dari tengah ke kiri/kanan, meredam seiring waktu dan jarak.
/// - Sisi kiri/kanan ikut menyempit saat melorot dan melebar saat memantul.
fn notch_wave(w: f32, h: f32, t: f32) -> Path {
    use std::f32::consts::{PI, TAU};

    let ear = (h * 0.35).min(10.0);
    let rb = (h * 0.5).min(22.0);
    let l = CX - w / 2.0;
    let r = CX + w / 2.0;
    let a = r - rb; // ujung kanan tepi bawah datar
    let b = l + rb; // ujung kiri tepi bawah datar
    let span = (a - b).max(1.0);

    // Ujung animasi mulus kembali ke bentuk normal.
    let fade = 1.0 - ((t / DROP_ANIM_SECS - 0.8) / 0.2).clamp(0.0, 1.0);
    // Gerak utama: pegas teredam (turun dulu, lalu memantul).
    let sag = (-3.4 * t).exp() * (TAU * 1.5 * t).sin();
    let max_down = (H as f32 - 2.0 - h).max(0.0);

    // Pergeseran vertikal tepi bawah pada posisi x.
    let off = |x: f32| -> f32 {
        let d = (x - CX).abs();
        let e = ((x - b) / span).clamp(0.0, 1.0);
        let win = (PI * e).sin().max(0.0).sqrt(); // 0 di sudut, 1 di tengah
        let body = sag * (-(d * d) / (2.0 * 110.0 * 110.0)).exp();
        let front = ((300.0 * t - d) / 50.0).clamp(0.0, 1.0); // riak baru ada setelah gelombang sampai
        let ripple = (-1.7 * t).exp() * (-d / 200.0).exp() * (TAU * (2.0 * t - d / 110.0)).sin() * front;
        let v = (body * DROP_AMP * 0.75 + ripple * DROP_AMP * 0.35) * win * fade;
        v.clamp(-h * 0.3, max_down)
    };

    // Sisi: positif = melebar keluar, negatif = menyempit.
    let sb = (-sag * DROP_AMP * 0.2 * fade).clamp(-5.0, 5.0);
    let mid = (ear + h - rb) / 2.0;

    let mut pb = PathBuilder::new();
    pb.move_to(l - ear, 0.0);
    pb.line_to(r + ear, 0.0);
    pb.quad_to(r, 0.0, r, ear);
    pb.quad_to(r + 2.0 * sb, mid, r, h - rb);
    pb.quad_to(r, h, a, h + off(a));

    let steps = ((span / 3.0).ceil() as i32).max(8);
    for i in 1..=steps {
        let x = a - span * i as f32 / steps as f32;
        pb.line_to(x, h + off(x));
    }

    pb.quad_to(l, h, l, h - rb);
    pb.quad_to(l - 2.0 * sb, mid, l, ear);
    pb.quad_to(l, 0.0, l - ear, 0.0);
    pb.close();
    pb.finish().unwrap_or_else(dummy_path)
}

pub(crate) fn draw(a: &mut App) {
    // Kalau panic sebelumnya terjadi saat gfx sedang dipinjam, buat ulang supaya tidak panic berulang.
    let mut g = match a.gfx.take() {
        Some(g) => g,
        None => Gfx {
            pm: Pixmap::new(W as u32, H as u32).unwrap(),
            text: Text::load(),
            mask: Mask::new(W as u32, H as u32),
            prev_box: (0, 0, W as usize, H as usize),
            dirty: (0, 0, W as usize, H as usize),
        },
    };
    let (w, h) = (a.w, a.h);
    let p = ((h - 40.0) / (EXP_H - 40.0)).clamp(0.0, 1.0);

    // Waktu animasi gelombang (None bila tidak ada drop yang sedang berjalan).
    let wave_t = a.drop_t();
    let dip = if wave_t.is_some() { DROP_AMP } else { 0.0 };

    // Only the area the notch can touch this frame (whole canvas while expanded).
    let (cw, ch) = (W as usize, H as usize);
    let cur: Box4 = if p > 0.04 {
        (0, 0, cw, ch)
    } else {
        let x1 = ((CX + w / 2.0 + 16.0).ceil().max(0.0) as usize).min(cw);
        let x0 = ((CX - w / 2.0 - 16.0).floor().max(0.0) as usize).min(x1);
        // Sisakan ruang ekstra di bawah notch supaya gelombang ikut tergambar.
        let y1 = ((h + 5.0 + dip * 1.6).ceil().max(0.0) as usize).min(ch);
        (x0, 0, x1, y1)
    };
    let dirty = union_box(g.prev_box, cur);
    g.clear_box(dirty);
    // FIX: backdrop kaca hanya di-blit ke `cur` (area yang di-mask di bawah), bukan `dirty`.
    // Kalau di-blit ke `dirty`, sisa area frame sebelumnya (di luar `cur`) berisi persegi
    // buram opak yang tidak ter-mask, sehingga kaca tampak keluar dari bentuk notch.
    if crate::glass::refresh(a.hwnd, a.wx) {
        crate::glass::blit(&mut g.pm, cur);
    }
    g.prev_box = cur;
    g.dirty = dirty;

    let shape = match wave_t {
        Some(t) => notch_wave(w, h, t),
        None => notch_path(w, h),
    };
    // The collapsed pill stays solid black; the glass tint and highlights fade in as the notch expands.
    g.glass_body(&shape, 1.0 - (1.0 - GLASS_ALPHA) * p, p);

    let ca = (1.0 - p * 2.0).clamp(0.0, 1.0) * ((h - 16.0) / 12.0).clamp(0.0, 1.0);
    if ca > 0.03 {
        a.draw_collapsed(&mut g, w, h, ca);
    }
    if p > 0.04 {
        let pos = a.sp - a.drag_dx / 160.0;
        let mut vis: Vec<(usize, f32, f32)> = Vec::new();
        for i in 0..N_SLIDES {
            let d = cyc(i as f32 - pos);
            if d.abs() >= 1.0 {
                continue;
            }
            vis.push((i, d * 48.0, (1.0 - d.abs()) * p));
        }
        for &(i, _, al) in &vis {
            match i {
                0 => a.music_glow(&mut g, &shape, w, h, al),
                _ => {}
            }
        }
        for &(i, ox, al) in &vis {
            match i {
                0 => a.slide_music(&mut g, ox, al),
                1 => a.slide_water(&mut g, ox, al),
                2 => a.slide_todo(&mut g, ox, al),
                3 => a.slide_shelf(&mut g, ox, al),
                _ => a.slide_note(&mut g, ox, al),
            }
        }
        a.draw_nav(&mut g, p);
        a.draw_dots(&mut g, pos, p);
    }
    apply_mask(&mut g, &shape, cur);
    a.gfx = Some(g);
}

pub(crate) fn glow(g: &mut Gfx, shape: &Path, h: f32, rgb: (u8, u8, u8), a: f32) {
    if a <= 0.01 {
        return;
    }
    let c = |al: f32| Color::from_rgba8(rgb.0, rgb.1, rgb.2, (al.clamp(0.0, 1.0) * 255.0) as u8);
    let stops = vec![GradientStop::new(0.0, c(a)), GradientStop::new(0.55, c(a * 0.28)), GradientStop::new(1.0, c(0.0))];
    if let Some(sh) =
        LinearGradient::new(Point::from_xy(0.0, h), Point::from_xy(0.0, h * 0.08), stops, SpreadMode::Pad, Transform::identity())
    {
        let mut p = Paint::default();
        p.shader = sh;
        p.anti_alias = true;
        g.fill(shape, &p);
    }
}

pub(crate) unsafe fn present(a: &mut App) {
    draw(a);
    let Some(g) = a.gfx.as_ref() else { return; };
    let dst = std::slice::from_raw_parts_mut(a.bits as *mut u8, (W * H * 4) as usize);
    // RGBA (premultiplied) -> BGRA, only for the rows/columns that changed.
    let (bx0, by0, bx1, by1) = g.dirty;
    let pw = W as usize;
    let pix = g.pm.data();
    for y in by0..by1.min(H as usize) {
        let (lo, hi) = ((y * pw + bx0) * 4, (y * pw + bx1.min(pw)) * 4);
        if hi <= lo {
            continue;
        }
        for (d, s) in dst[lo..hi].chunks_exact_mut(4).zip(pix[lo..hi].chunks_exact(4)) {
            d[0] = s[2];
            d[1] = s[1];
            d[2] = s[0];
            d[3] = s[3];
        }
    }
    if GLASS_BLUR {
        apply_blur(HWND(a.hwnd as *mut c_void), blur_rects(g));
    }
    let size = SIZE { cx: W, cy: H };
    let src = POINT { x: 0, y: 0 };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    let _ = UpdateLayeredWindow(
        HWND(a.hwnd as *mut c_void),
        HDC::default(),
        None,
        Some(&size),
        HDC(a.mem_dc as *mut c_void),
        Some(&src),
        COLORREF(0),
        Some(&blend),
        ULW_ALPHA,
    );
}

impl App {
    fn draw_collapsed(&self, g: &mut Gfx, w: f32, h: f32, ca: f32) {
        if self.media.playing {
            let x0 = CX - w / 2.0;
            self.icon_at(g, x0 + 14.0, (h - 20.0) / 2.0, 20.0, 5.0, ca);
            for i in 0..4 {
                let bh = 4.0 + 12.0 * (0.5 + 0.5 * (self.t * 6.0 + i as f32 * 1.7).sin());
                g.rect(x0 + w - 34.0 + i as f32 * 6.0, h / 2.0 - bh / 2.0, 3.0, bh, &acc(ca));
            }
            let (rl, rr) = (x0 + 44.0, x0 + w - 44.0);
            let rw = rr - rl;
            let title = self.media.title.as_str();
            let size = 12.0;
            let tw = g.width(title, size);
            let y = ty(h, size);
            if tw <= rw {
                g.txt(title, rl + (rw - tw) / 2.0, y, size, [255, 255, 255], ca, rw + 2.0);
            } else {
                let cycle = tw + MARQUEE_GAP;
                let period = MARQUEE_PAUSE + cycle / MARQUEE_SPEED;
                let e = (self.t - self.title_t0).max(0.0) % period;
                let off = if e < MARQUEE_PAUSE { 0.0 } else { (e - MARQUEE_PAUSE) * MARQUEE_SPEED };
                let (cl, cr) = (rl - 6.0, rr + 6.0);
                g.txt_clip(title, rl - off, y, size, [255, 255, 255], ca, cl, cr, 6.0);
                g.txt_clip(title, rl - off + cycle, y, size, [255, 255, 255], ca, cl, cr, 6.0);
            }
        } else if let Some(st) = self.splash_t() {
            // Startup splash: eye + glow, cross-fading into the clock during the last SPLASH_FADE seconds.
            self.draw_eye(g, w, h, ca);
            let cf = ((st - (SPLASH_SECS - SPLASH_FADE)) / SPLASH_FADE).clamp(0.0, 1.0);
            if cf > 0.0 {
                self.clock_text(g, w, h, ca * cf);
            }
        } else {
            match self.idle_phase() {
                IdlePhase::Clock => self.draw_clock_collapsed(g, w, h, ca),
                IdlePhase::Eye => self.draw_eye(g, w, h, ca),
            }
        }
    }
}