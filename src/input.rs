//! Input mouse: klik per slide dan context menu.

use super::*;

pub(crate) unsafe fn click(x: f32, y: f32) {
    with_app(|a| {
        if !a.ready() {
            return;
        }
        let near = |c: (f32, f32)| ((x - c.0).powi(2) + (y - c.1).powi(2)).sqrt() < 16.0;
        if near(NAV_L) || near(NAV_R) {
            if a.editing {
                return;
            }
            a.step(if near(NAV_L) { -1 } else { 1 });
            a.last = Instant::now();
            present(a);
            schedule(a);
            return;
        }
        if y >= EXP_H - 22.0 && !a.editing {
            for i in 0..N_SLIDES {
                if (x - dot_x(i)).abs() < 8.0 {
                    a.show(i);
                    a.last = Instant::now();
                    schedule(a);
                    return;
                }
            }
        }
        match a.cur() {
            0 => {
                let d = |px: f32, py: f32| ((x - px).powi(2) + (y - py).powi(2)).sqrt();
                if d(MCX - 56.0, MCY) < 17.0 {
                    let _ = a.tx.send(Cmd::Prev);
                } else if d(MCX, MCY) < 19.0 {
                    let _ = a.tx.send(Cmd::Toggle);
                } else if d(MCX + 56.0, MCY) < 17.0 {
                    let _ = a.tx.send(Cmd::Next);
                } else if a.media.dur > 0 && in_rect(x, y, MX, 88.0, MW, 16.0) {
                    let _ = a.tx.send(Cmd::Seek(((x - MX) / MW).clamp(0.0, 1.0)));
                }
            }
            1 => {
                if in_rect(x, y, BX, 42.0, 112.0, 34.0) {
                    a.water_add(250);
                } else if in_rect(x, y, BX, 84.0, 112.0, 34.0) {
                    a.water_add(500);
                }
                present(a);
            }
            2 => {
                if a.editing {
                    return;
                }
                if in_rect(x, y, LX, 116.0, 92.0, 28.0) {
                    a.begin_edit(None);
                    schedule(a);
                } else if in_rect(x, y, LX + 98.0, 116.0, 92.0, 28.0) {
                    a.edit_mode = !a.edit_mode;
                } else {
                    let (rx, rw, y0, max) = rows_geom(false);
                    let order = a.todo_order();
                    for vi in 0..max {
                        let idx = a.todo_scroll + vi;
                        if idx >= order.len() {
                            break;
                        }
                        let ri = order[idx];
                        let ry = y0 + vi as f32 * ROW_P;
                        if in_rect(x, y, rx, ry, rw, ROW_H) {
                            if a.edit_mode {
                                if x > rx + rw - 24.0 {
                                    a.todos.remove(ri);
                                    save_todos(&a.todos);
                                    a.fix_scroll();
                                } else {
                                    a.begin_edit(Some(ri));
                                    schedule(a);
                                }
                            } else {
                                a.todos[ri].0 = !a.todos[ri].0;
                                save_todos(&a.todos);
                            }
                            break;
                        }
                    }
                }
                present(a);
            }
            3 => {
                let mut hit: Option<(usize, bool)> = None;
                for i in 0..a.shelf.len() {
                    let tx = LX + i as f32 * (SH_W + SH_GAP);
                    if in_rect(x, y, tx, SH_Y, SH_W, SH_H) {
                        let (bx, by) = (tx + SH_W - 10.0, SH_Y + 10.0);
                        hit = Some((i, ((x - bx).powi(2) + (y - by).powi(2)).sqrt() < 11.0));
                        break;
                    }
                }
                if let Some((i, close)) = hit {
                    // Remove on "x", or when the file/folder no longer exists; otherwise open it.
                    if close || !std::path::Path::new(&a.shelf[i].path).exists() {
                        a.shelf.remove(i);
                        save_shelf(&a.shelf);
                    } else {
                        open_path(&a.shelf[i].path);
                    }
                }
                present(a);
            }
            4 => {
                let in_card = in_rect(x, y, LX, NOTE_Y, CW, NOTE_H);
                if a.note_mode {
                    if !in_card {
                        a.finish_edit(false, true);
                    }
                } else if !a.note.is_empty() && in_rect(x, y, RX - 56.0, 8.0, 56.0, 24.0) {
                    a.note.clear();
                    save_note(&a.note);
                } else if in_card {
                    a.begin_note();
                    schedule(a);
                }
                present(a);
            }
            _ => {}
        }
    });
}

pub(crate) unsafe fn context_menu(hwnd: HWND, goal: u32, remind: u32) -> i32 {
    crate::menu::context_menu(hwnd, goal, remind)
}