//! Window procedure Win32 (pesan mouse, timer, hotkey, drop file, dll).

use super::*;

use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_SHIFT};

/// Panic di dalam extern "system" mematikan proses tanpa pesan, jadi dibungkus catch_unwind.
/// Detail panic ditulis ke %APPDATA%\poch\crash.log.
pub(crate) unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match std::panic::catch_unwind(|| wndproc_inner(hwnd, msg, wp, lp)) {
        Ok(r) => r,
        Err(_) => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe fn wndproc_inner(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            tick(wp.0);
            LRESULT(0)
        }
        WM_APP => {
            with_app(|a| {
                let was = a.media.playing;
                a.media = a.shared.lock().unwrap().clone();
                let key = a.media.key();
                if key != a.art_key {
                    a.art = None;
                    a.art_key = key;
                    a.title_t0 = a.t;
                }
                let slot = a.art_shared.lock().unwrap().take();
                if let Some(d) = slot {
                    if d.key == a.art_key {
                        a.set_art(&d.rgba);
                    }
                }
                if a.media.playing && !was && !a.expanded() {
                    a.show(0);
                }
                a.last = Instant::now();
                present(a);
                schedule(a);
            });
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lp.0 & 0xffff) as i16 as f32;
            let y = ((lp.0 >> 16) & 0xffff) as i16 as f32;
            let _ = SetCapture(hwnd);

            // Press inside the note card while editing: caret placement / drag-selection,
            // not a swipe or a click on the slide.
            let shift = (GetKeyState(VK_SHIFT.0 as i32) as u16 & 0x8000) != 0;
            let mut in_note = false;
            with_app(|a| {
                in_note = a.note_mouse_down(x, y, shift);
            });
            if in_note {
                return LRESULT(0);
            }

            with_app(|a| {
                a.drag = Some((x, y));
                a.drag_dx = 0.0;
                a.shelf_drag = None;
                // Deteksi kalau tombol ditekan di atas item shelf (bukan tombol ×): siapkan drag-out.
                if a.ready() && !a.editing && a.cur() == 3 {
                    for i in 0..a.shelf.len() {
                        let tx = LX + i as f32 * (SH_W + SH_GAP);
                        if in_rect(x, y, tx, SH_Y, SH_W, SH_H) {
                            let (bx, by) = (tx + SH_W - 10.0, SH_Y + 10.0);
                            let d2 = (x - bx).powi(2) + (y - by).powi(2);
                            if d2.sqrt() >= 11.0 {
                                a.shelf_drag = Some((i, x, y));
                            }
                            break;
                        }
                    }
                }
            });
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let x = (lp.0 & 0xffff) as i16 as f32;
            let y = ((lp.0 >> 16) & 0xffff) as i16 as f32;
            let mut start: Option<(usize, String)> = None;
            with_app(|a| {
                // Drag-selecting text in the note: extend the selection (scrolls with the caret).
                if a.nv.sel_drag {
                    a.note_mouse_drag(x, y);
                    return;
                }
                // Bila user sudah cukup jauh dari posisi awal shelf: mulai OLE drag-out.
                if let Some((idx, sx, sy)) = a.shelf_drag {
                    let d2 = (x - sx).powi(2) + (y - sy).powi(2);
                    if d2 >= 36.0 {
                        if let Some(it) = a.shelf.get(idx) {
                            start = Some((idx, it.path.clone()));
                        }
                        a.shelf_drag = None;
                        a.drag = None;
                        a.drag_dx = 0.0;
                    }
                }
                if let Some((sx, _)) = a.drag {
                    let dx = x - sx;
                    let nd = if dx.abs() > 8.0 { dx } else { 0.0 };
                    if nd != a.drag_dx {
                        a.drag_dx = nd;
                        present(a);
                    }
                }
            });
            if let Some((idx, path)) = start {
                // DoDragDrop punya loop sendiri; keluar dulu dari lock & capture.
                let _ = ReleaseCapture();
                let moved = dragdrop::start_shelf_drag(&path);
                with_app(|a| {
                    if moved && idx < a.shelf.len() {
                        a.shelf.remove(idx);
                        save_shelf(&a.shelf);
                    }
                    a.last = Instant::now();
                    present(a);
                    schedule(a);
                });
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let _ = ReleaseCapture();
            let mut click_at: Option<(f32, f32)> = None;
            with_app(|a| {
                a.shelf_drag = None;
                a.nv.sel_drag = false;
                if let Some((sx, sy)) = a.drag.take() {
                    let dx = a.drag_dx;
                    a.drag_dx = 0.0;
                    if dx.abs() >= 40.0 && !a.editing {
                        a.sp -= dx / 160.0;
                        a.sv = 0.0;
                        a.step(if dx < 0.0 { 1 } else { -1 });
                        a.last = Instant::now();
                    } else if dx.abs() < 8.0 {
                        click_at = Some((sx, sy));
                    } else {
                        a.sp -= dx / 160.0;
                        a.sv = 0.0;
                        a.last = Instant::now();
                    }
                    present(a);
                    schedule(a);
                }
            });
            if let Some((x, y)) = click_at {
                click(x, y);
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let d = ((wp.0 >> 16) & 0xffff) as i16 as i32;
            with_app(|a| {
                if d == 0 || !a.ready() {
                    return;
                }
                let vertical = msg == WM_MOUSEWHEEL;
                // Scrap slide: wheel / trackpad scrolls a long note instead of switching slides.
                if vertical && a.cur() == 4 && a.note_wheel(d) {
                    present(a);
                    return;
                }
                if vertical && a.cur() == 2 && !a.editing && a.todos.len() > 3 {
                    if a.last_wheel.elapsed() < Duration::from_millis(90) {
                        return;
                    }
                    a.last_wheel = Instant::now();
                    let max = a.todos.len() - 3;
                    a.todo_scroll = if d > 0 { a.todo_scroll.saturating_sub(1) } else { (a.todo_scroll + 1).min(max) };
                    present(a);
                    return;
                }
                if a.editing {
                    return;
                }
                if a.last_wheel.elapsed() < Duration::from_millis(260) {
                    return;
                }
                let dir = if vertical {
                    if d > 0 { -1 } else { 1 }
                } else if d > 0 {
                    1
                } else {
                    -1
                };
                a.last_wheel = Instant::now();
                a.step(dir);
                a.last = Instant::now();
                schedule(a);
            });
            LRESULT(0)
        }
        WM_KEYDOWN => {
            // Arrows, Home/End, PageUp/PageDown, Delete (+ Shift / Ctrl) while editing the note.
            let vk = wp.0 as u32;
            let shift = (GetKeyState(VK_SHIFT.0 as i32) as u16 & 0x8000) != 0;
            let ctrl = (GetKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000) != 0;
            let mut handled = false;
            with_app(|a| {
                if a.editing && a.note_mode {
                    handled = a.note_key(vk, shift, ctrl);
                }
            });
            if handled {
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, msg, wp, lp)
            }
        }
        WM_CHAR => {
            let c = wp.0 as u32;
            with_app(|a| {
                if !a.editing {
                    if c == 'h' as u32 || c == 'H' as u32 {
                        toggle_idle_phase(a);
                    }
                    return;
                }
                if a.note_mode {
                    a.note_char(c);
                    return;
                }
                match c {
                    13 => a.finish_edit(true, true),
                    27 => a.finish_edit(false, true),
                    8 => {
                        a.input.pop();
                    }
                    c if c >= 32 => {
                        if let Some(ch) = char::from_u32(c) {
                            if a.input.chars().count() < 60 {
                                a.input.push(ch);
                            }
                        }
                    }
                    _ => {}
                }
                a.edit_start = Instant::now();
                a.caret_on = true;
                a.last = Instant::now();
                present(a);
                schedule(a);
            });
            LRESULT(0)
        }
        WM_KILLFOCUS => {
            with_app(|a| {
                a.nv.sel_drag = false;
                if a.editing {
                    a.finish_edit(false, false);
                    a.last = Instant::now();
                    present(a);
                    schedule(a);
                }
            });
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            let mut goal = 2000;
            let mut remind = 60;
            with_app(|a| {
                a.menu_open = true;
                goal = a.water.goal;
                remind = a.water.remind_min;
            });
            let cmd = context_menu(hwnd, goal, remind);
            with_app(|a| {
                a.menu_open = false;
                match cmd {
                    10 => {
                        a.water.ml = 0;
                        a.water.save();
                    }
                    11 => a.water_add(-250),
                    20..=23 => {
                        a.water.goal = [1500, 2000, 2500, 3000][(cmd - 20) as usize];
                        a.water.save();
                    }
                    30..=33 => {
                        a.water.remind_min = [30, 60, 90, 0][(cmd - 30) as usize];
                        a.water.reset_timer();
                        a.water.save();
                    }
                    _ => {}
                }
                a.last = Instant::now();
                present(a);
                schedule(a);
            });
            if cmd == 1 {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_HOTKEY => {
            if wp.0 as i32 == HOTKEY_ID {
                with_app(|a| toggle_idle_phase(a));
            }
            LRESULT(0)
        }
        WM_DROPFILES => {
            let hdrop = HDROP(wp.0 as *mut c_void);
            let n = DragQueryFileW(hdrop, 0xFFFF_FFFF, None);
            let mut paths: Vec<String> = Vec::new();
            for i in 0..n {
                let len = DragQueryFileW(hdrop, i, None) as usize;
                let mut buf = vec![0u16; len + 1];
                let got = DragQueryFileW(hdrop, i, Some(&mut buf)) as usize;
                paths.push(String::from_utf16_lossy(&buf[..got.min(buf.len())]));
            }
            DragFinish(hdrop);
            with_app(|a| {
                a.shelf_add(paths);
                if !a.editing {
                    a.show(3);
                }
                a.peek_until = Some(Instant::now() + Duration::from_secs(4));
                a.last = Instant::now();
                present(a);
                schedule(a);
            });
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(3),
        WM_DESTROY => {
            with_app(|a| {
                if a.note_mode {
                    save_note(&a.note);
                }
            });
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}