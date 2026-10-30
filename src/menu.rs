//! Context menu gelap buatan sendiri: window layered yang digambar dengan tiny-skia.
//!
//! Window menu TIDAK diaktifkan (WS_EX_NOACTIVATE), karena app utama juga tidak pernah jadi
//! foreground window dan Windows menolak/membatalkan aktivasi lewat SetForegroundWindow.
//! Karena itu input (hover, klik, Esc, panah, Enter) dibaca lewat polling timer
//! (GetCursorPos + GetAsyncKeyState), sehingga tidak bergantung pada fokus atau mouse capture.
//! Handler pesan mouse tetap ada sebagai jalur cadangan.

use super::*;

use std::cell::RefCell;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_DOWN, VK_ESCAPE, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON, VK_RETURN, VK_UP,
};

const MENU_W: f32 = 228.0;
const PAD: f32 = 6.0;
const ITEM_H: f32 = 30.0;
const SEP_H: f32 = 11.0;
const R_OUT: f32 = 14.0;
const FONT: f32 = 13.0;
const POLL_TIMER: usize = 1;
const POLL_MS: u32 = 15;

enum Entry {
    Item { id: i32, label: &'static str, checked: bool },
    Sep,
}

struct Menu {
    /// Entri beserta y atasnya.
    entries: Vec<(Entry, f32)>,
    w: i32,
    h: i32,
    /// Posisi window di layar.
    x0: i32,
    y0: i32,
    hover: Option<usize>,
    result: i32,
    done: bool,
    // Status polling (deteksi tepi tombol/tombol keyboard).
    last_pt: (i32, i32),
    pl: bool,
    pr: bool,
    pm: bool,
    keys: [bool; 4],
    g: Gfx,
    dc: HDC,
    bmp: HBITMAP,
    bits: *mut c_void,
}

thread_local! {
    static MENU: RefCell<Option<Menu>> = RefCell::new(None);
}

fn with_menu<R>(f: impl FnOnce(&mut Menu) -> R) -> Option<R> {
    MENU.with(|c| c.try_borrow_mut().ok().and_then(|mut m| m.as_mut().map(f)))
}

fn finish(id: i32) {
    with_menu(|m| m.close(id));
}

fn key_down(vk: u16) -> bool {
    unsafe { (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0 }
}

impl Menu {
    /// Hasil pertama menang; pemanggilan berikutnya diabaikan.
    fn close(&mut self, id: i32) {
        if !self.done {
            self.result = id;
            self.done = true;
        }
    }

    fn hit(&self, x: f32, y: f32) -> Option<usize> {
        self.entries.iter().position(|(e, ey)| {
            matches!(e, Entry::Item { .. }) && in_rect(x, y, PAD, *ey, MENU_W - 2.0 * PAD, ITEM_H)
        })
    }

    fn id_at(&self, i: usize) -> i32 {
        match &self.entries[i].0 {
            Entry::Item { id, .. } => *id,
            Entry::Sep => 0,
        }
    }

    fn step(&mut self, down: bool) {
        let items: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, p)| matches!(p.0, Entry::Item { .. }))
            .map(|(i, _)| i)
            .collect();
        let n = items.len();
        if n == 0 {
            return;
        }
        let pos = self.hover.and_then(|h| items.iter().position(|&i| i == h));
        let next = match (pos, down) {
            (None, true) => 0,
            (None, false) => n - 1,
            (Some(p), true) => (p + 1) % n,
            (Some(p), false) => (p + n - 1) % n,
        };
        self.hover = Some(items[next]);
    }

    fn paint_frame(&mut self) {
        let (w, h) = (self.w as f32, self.h as f32);
        let g = &mut self.g;
        g.pm.fill(Color::TRANSPARENT);
        let panel = rrect(0.5, 0.5, w - 1.0, h - 1.0, R_OUT);
        g.fill(&panel, &paint(20, 20, 22, 0.97));
        g.stroke(&panel, &paint(255, 255, 255, 0.10), 1.0);

        let rw = w - 2.0 * PAD;
        for (i, (e, y)) in self.entries.iter().enumerate() {
            let y = *y;
            match e {
                Entry::Sep => {
                    g.rect(PAD + 8.0, y + SEP_H / 2.0, rw - 16.0, 1.0, &paint(255, 255, 255, 0.09));
                }
                Entry::Item { label, checked, .. } => {
                    if self.hover == Some(i) {
                        g.fill(&rrect(PAD, y, rw, ITEM_H, R_OUT - PAD), &paint(255, 255, 255, 0.09));
                    }
                    if *checked {
                        let (cx, cy) = (PAD + 16.0, y + ITEM_H / 2.0);
                        let p = acc(1.0);
                        g.line(cx - 4.5, cy + 0.2, cx - 1.5, cy + 3.4, 1.8, &p);
                        g.line(cx - 1.5, cy + 3.4, cx + 4.5, cy - 3.4, 1.8, &p);
                    }
                    g.txt(label, PAD + 32.0, y + ty(ITEM_H, FONT), FONT, [236, 236, 238], 1.0, rw - 40.0);
                }
            }
        }
    }

    fn copy_out(&self) {
        let len = (self.w * self.h * 4) as usize;
        let dst = unsafe { std::slice::from_raw_parts_mut(self.bits as *mut u8, len) };
        for (d, s) in dst.chunks_exact_mut(4).zip(self.g.pm.data().chunks_exact(4)) {
            d[0] = s[2];
            d[1] = s[1];
            d[2] = s[0];
            d[3] = s[3];
        }
    }
}

unsafe fn build(goal: u32, remind: u32) -> Option<Menu> {
    let it = |id: i32, label: &'static str, checked: bool| Entry::Item { id, label, checked };
    let list = vec![
        it(11, "Remove 250 ml", false),
        it(10, "Reset today's water", false),
        Entry::Sep,
        it(20, "Daily goal 1.5 L", goal == 1500),
        it(21, "Daily goal 2.0 L", goal == 2000),
        it(22, "Daily goal 2.5 L", goal == 2500),
        it(23, "Daily goal 3.0 L", goal == 3000),
        Entry::Sep,
        it(30, "Remind every 30 min", remind == 30),
        it(31, "Remind every 60 min", remind == 60),
        it(32, "Remind every 90 min", remind == 90),
        it(33, "Reminders off", remind == 0),
        Entry::Sep,
        it(1, "Exit", false),
    ];

    let mut y = PAD;
    let mut entries = Vec::with_capacity(list.len());
    for e in list {
        let eh = if matches!(e, Entry::Sep) { SEP_H } else { ITEM_H };
        entries.push((e, y));
        y += eh;
    }
    let (w, h) = (MENU_W as i32, (y + PAD).ceil() as i32);

    let screen = GetDC(None);
    let dc = CreateCompatibleDC(screen);
    ReleaseDC(None, screen);
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
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

    let g = Gfx {
        pm: Pixmap::new(w as u32, h as u32)?,
        text: Text::load(),
        mask: None,
        prev_box: (0, 0, 0, 0),
        dirty: (0, 0, 0, 0),
    };
    Some(Menu {
        entries,
        w,
        h,
        x0: 0,
        y0: 0,
        hover: None,
        result: 0,
        done: false,
        last_pt: (i32::MIN, i32::MIN),
        pl: false,
        pr: false,
        pm: false,
        keys: [false; 4],
        g,
        dc,
        bmp,
        bits,
    })
}

unsafe fn redraw(hwnd: HWND) {
    let Some((dc, w, h)) = with_menu(|m| {
        m.paint_frame();
        m.copy_out();
        (m.dc, m.w, m.h)
    }) else {
        return;
    };
    let size = SIZE { cx: w, cy: h };
    let src = POINT { x: 0, y: 0 };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    let _ = UpdateLayeredWindow(hwnd, HDC::default(), None, Some(&size), dc, Some(&src), COLORREF(0), Some(&blend), ULW_ALPHA);
}

/// Dipanggil tiap POLL_MS: membaca kursor, tombol mouse, dan tombol keyboard langsung dari sistem.
unsafe fn poll(hwnd: HWND) {
    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    let l = key_down(VK_LBUTTON.0);
    let r = key_down(VK_RBUTTON.0);
    let md = key_down(VK_MBUTTON.0);
    let ks = [key_down(VK_ESCAPE.0), key_down(VK_UP.0), key_down(VK_DOWN.0), key_down(VK_RETURN.0)];

    let changed = with_menu(|m| {
        if m.done {
            return false;
        }
        let x = (pt.x - m.x0) as f32;
        let y = (pt.y - m.y0) as f32;
        let inside = x >= 0.0 && y >= 0.0 && x < m.w as f32 && y < m.h as f32;
        let hit = if inside { m.hit(x, y) } else { None };

        let mut changed = false;
        // Hover hanya diperbarui kalau kursor bergerak, supaya navigasi keyboard tidak tertimpa.
        if (pt.x, pt.y) != m.last_pt {
            m.last_pt = (pt.x, pt.y);
            if hit != m.hover {
                m.hover = hit;
                changed = true;
            }
        }

        let pressed = (l && !m.pl) || (r && !m.pr) || (md && !m.pm);
        let released = (!l && m.pl) || (!r && m.pr);
        if pressed && !inside {
            m.close(0);
        } else if released {
            if let Some(i) = hit {
                let id = m.id_at(i);
                m.close(id);
            }
        }
        m.pl = l;
        m.pr = r;
        m.pm = md;

        if !m.done {
            if ks[0] && !m.keys[0] {
                m.close(0);
            } else if ks[1] && !m.keys[1] {
                m.step(false);
                changed = true;
            } else if ks[2] && !m.keys[2] {
                m.step(true);
                changed = true;
            } else if ks[3] && !m.keys[3] {
                if let Some(i) = m.hover {
                    let id = m.id_at(i);
                    m.close(id);
                }
            }
        }
        m.keys = ks;
        changed
    })
    .unwrap_or(false);

    if changed {
        redraw(hwnd);
    }
}

unsafe extern "system" fn menu_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match std::panic::catch_unwind(|| menu_proc_inner(hwnd, msg, wp, lp)) {
        Ok(r) => r,
        Err(_) => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe fn menu_proc_inner(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            if wp.0 == POLL_TIMER {
                poll(hwnd);
            }
            LRESULT(0)
        }
        // Jangan pernah mengambil fokus dari app lain.
        WM_MOUSEACTIVATE => LRESULT(3),
        // Jalur cadangan lewat pesan mouse (poll biasanya sudah menanganinya lebih dulu).
        WM_MOUSEMOVE => {
            let x = (lp.0 & 0xffff) as i16 as f32;
            let y = ((lp.0 >> 16) & 0xffff) as i16 as f32;
            let moved = with_menu(|m| {
                let h = m.hit(x, y);
                let changed = h != m.hover;
                m.hover = h;
                changed
            })
            .unwrap_or(false);
            if moved {
                redraw(hwnd);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP | WM_RBUTTONUP => {
            let x = (lp.0 & 0xffff) as i16 as f32;
            let y = ((lp.0 >> 16) & 0xffff) as i16 as f32;
            if let Some(id) = with_menu(|m| m.hit(x, y).map(|i| m.id_at(i))).flatten() {
                finish(id);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN => LRESULT(0),
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// Menampilkan menu di posisi kursor dan menunggu pilihan. Mengembalikan id perintah, 0 bila dibatalkan.
pub(crate) unsafe fn context_menu(owner: HWND, goal: u32, remind: u32) -> i32 {
    let Some(mut menu) = build(goal, remind) else { return 0; };
    let (mw, mh) = (menu.w, menu.h);

    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
    let x = pt.x.min(sw - mw - 8).max(0);
    let y = if pt.y + mh + 8 > sh { (pt.y - mh).max(0) } else { pt.y };

    // Status awal tombol/keyboard diambil sekarang, jadi tombol yang masih ditekan saat menu
    // dibuka tidak dihitung sebagai klik baru.
    menu.x0 = x;
    menu.y0 = y;
    menu.last_pt = (i32::MIN, i32::MIN);
    menu.pl = key_down(VK_LBUTTON.0);
    menu.pr = key_down(VK_RBUTTON.0);
    menu.pm = key_down(VK_MBUTTON.0);
    menu.keys = [key_down(VK_ESCAPE.0), key_down(VK_UP.0), key_down(VK_DOWN.0), key_down(VK_RETURN.0)];
    MENU.with(|c| *c.borrow_mut() = Some(menu));

    let hinst = GetModuleHandleW(None).unwrap_or_default();
    let class = w!("PochMenu");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(menu_proc),
        hInstance: hinst.into(),
        lpszClassName: class,
        hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
        ..Default::default()
    };
    RegisterClassW(&wc);

    // Dimiliki window notch, jadi Windows selalu menaruhnya di atas notch.
    let created = CreateWindowExW(
        WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
        class,
        w!(""),
        WS_POPUP,
        x,
        y,
        mw,
        mh,
        owner,
        None,
        hinst,
        None,
    );

    if let Ok(hwnd) = created {
        redraw(hwnd);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = SetTimer(hwnd, POLL_TIMER, POLL_MS, None);

        let mut msg = MSG::default();
        while !with_menu(|m| m.done).unwrap_or(true) {
            if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
                PostQuitMessage(msg.wParam.0 as i32);
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let _ = KillTimer(hwnd, POLL_TIMER);
        let _ = DestroyWindow(hwnd);
    }

    let mut result = 0;
    if let Some(m) = MENU.with(|c| c.borrow_mut().take()) {
        result = m.result;
        let _ = DeleteObject(HGDIOBJ(m.bmp.0));
        let _ = DeleteDC(m.dc);
    }
    result
}