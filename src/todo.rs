//! Fitur To-do: daftar tugas, editor teks, slide To-do.

use super::*;

const LP: f32 = LX + 204.0;
pub(crate) const ROW_H: f32 = 30.0;
pub(crate) const ROW_P: f32 = 35.0;

pub(crate) fn rows_geom(editing: bool) -> (f32, f32, f32, usize) {
    if editing {
        (LX, CW - 10.0, 78.0, 2)
    } else {
        (LP, RX - 10.0 - LP, 44.0, 3)
    }
}

pub(crate) fn load_todos() -> Vec<(bool, String)> {
    match std::fs::read_to_string(data_path("todo.txt")) {
        Ok(s) => s
            .lines()
            .filter_map(|l| l.split_once('|').map(|(d, t)| (d == "1", t.to_string())))
            .collect(),
        Err(_) => {
            let d = vec![(false, "Tap Add Task to create a task".to_string())];
            save_todos(&d);
            d
        }
    }
}

pub(crate) fn save_todos(t: &[(bool, String)]) {
    let s: String = t.iter().map(|(d, x)| format!("{}|{}\n", if *d { 1 } else { 0 }, x)).collect();
    write_data("todo.txt", s);
}

impl App {
    pub(crate) fn todo_order(&self) -> Vec<usize> {
        let mut v: Vec<usize> = (0..self.todos.len()).filter(|&i| !self.todos[i].0).collect();
        v.extend((0..self.todos.len()).filter(|&i| self.todos[i].0));
        v
    }

    pub(crate) fn fix_scroll(&mut self) {
        let max = self.todos.len().saturating_sub(3);
        self.todo_scroll = self.todo_scroll.min(max);
    }

    pub(crate) unsafe fn begin_edit(&mut self, idx: Option<usize>) {
        self.editing = true;
        self.edit_idx = idx;
        self.input = idx.map(|i| self.todos[i].1.clone()).unwrap_or_default();
        self.caret_on = true;
        self.edit_start = Instant::now();
        self.prev_fg = GetForegroundWindow().0 as isize;
        let hwnd = HWND(self.hwnd as *mut c_void);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(hwnd);
    }

    pub(crate) unsafe fn finish_edit(&mut self, commit: bool, restore: bool) {
        if self.note_mode {
            // The note is edited in place, so Esc / click away / focus loss all keep the text.
            save_note(&self.note);
            self.note_mode = false;
            self.editing = false;
            if restore && self.prev_fg != 0 {
                let _ = SetForegroundWindow(HWND(self.prev_fg as *mut c_void));
            }
            return;
        }
        if commit {
            let t = self.input.trim().to_string();
            if !t.is_empty() {
                match self.edit_idx {
                    Some(i) if i < self.todos.len() => self.todos[i].1 = t,
                    _ => self.todos.push((false, t)),
                }
                save_todos(&self.todos);
            }
        }
        self.editing = false;
        self.edit_idx = None;
        self.input.clear();
        if restore && self.prev_fg != 0 {
            let _ = SetForegroundWindow(HWND(self.prev_fg as *mut c_void));
        }
    }

    fn todo_row(&self, g: &mut Gfx, x: f32, y: f32, w: f32, ri: usize, al: f32) {
        let (dn, s) = (self.todos[ri].0, &self.todos[ri].1);
        if dn {
            g.fill(&rrect(x, y, w, ROW_H, 10.0), &acc(0.20 * al));
        } else {
            g.fill(&rrect(x, y, w, ROW_H, 10.0), &paint(255, 255, 255, 0.08 * al));
        }
        let (cx, cy) = (x + 17.0, y + ROW_H / 2.0);
        if dn {
            g.circle(cx, cy, 8.0, &acc(al));
            let wh = paint(255, 255, 255, al);
            g.line(cx - 3.6, cy + 0.4, cx - 1.0, cy + 3.0, 1.9, &wh);
            g.line(cx - 1.0, cy + 3.0, cx + 3.8, cy - 2.8, 1.9, &wh);
        } else {
            if let Some(ring) = PathBuilder::from_circle(cx, cy, 7.35) {
                g.stroke(&ring, &paint(255, 255, 255, 0.32 * al), 1.3);
            }
        }
        let col = if dn { [150, 150, 150] } else { [242, 242, 242] };
        let tx = x + 33.0;
        let maxw = w - 33.0 - 10.0 - if self.edit_mode { 24.0 } else { 0.0 };
        g.txt(s, tx, y + ty(ROW_H, 13.0), 13.0, col, al, maxw);
        if dn {
            let tw = g.width(s, 13.0).min(maxw);
            g.rect(tx, cy, tw, 1.3, &paint(175, 175, 175, 0.9 * al));
        }
        if self.edit_mode {
            let (bx, by) = (x + w - 17.0, cy);
            g.circle(bx, by, 8.0, &acc(0.9 * al));
            let wh = paint(255, 255, 255, al);
            g.line(bx - 2.8, by - 2.8, bx + 2.8, by + 2.8, 1.6, &wh);
            g.line(bx - 2.8, by + 2.8, bx + 2.8, by - 2.8, 1.6, &wh);
        }
    }

    fn pill(&self, g: &mut Gfx, x: f32, y: f32, label: &str, edit_icon: bool, active: bool, al: f32) {
        let bg = if active { acc(0.95 * al) } else { paint(255, 255, 255, 0.11 * al) };
        g.fill(&rrect(x, y, 92.0, 28.0, 14.0), &bg);
        let (ix, iy) = (x + 16.0, y + 14.0);
        let wh = paint(255, 255, 255, al);
        if edit_icon {
            g.line(ix - 4.0, iy + 4.5, ix + 4.0, iy - 4.0, 2.4, &wh);
            g.poly(&[(ix - 5.5, iy + 6.0), (ix - 5.0, iy + 2.5), (ix - 2.5, iy + 5.0)], &wh);
        } else {
            g.circle(ix, iy, 7.0, &wh);
            let bk = paint(0, 0, 0, al);
            g.rect(ix - 3.5, iy - 0.8, 7.0, 1.6, &bk);
            g.rect(ix - 0.8, iy - 3.5, 1.6, 7.0, &bk);
        }
        g.txt(label, x + 30.0, y + ty(28.0, 12.0), 12.0, [255, 255, 255], al, 60.0);
    }

    pub(crate) fn slide_todo(&self, g: &mut Gfx, ox: f32, al: f32) {
        let x = LX + ox;
        g.txtb("Todo", x, 12.0, 15.0, [255, 255, 255], al, 80.0);
        let tw = g.width_ex("Todo", 15.0, true);
        g.txtb("List", x + tw, 12.0, 15.0, ACCENT, al, 80.0);
        let st = now_tm();
        let time = format!("{:02}:{:02}", st.wHour, st.wMinute);
        let date = format!(
            "{} {} {} {}",
            DAYS_S[(st.wDayOfWeek as usize) % 7],
            st.wDay,
            MONTHS[((st.wMonth as usize).max(1) - 1) % 12],
            st.wYear
        );
        let timew = g.width_ex(&time, 13.0, true);
        let datew = g.width(&date, 13.0);
        let tx = RX + ox - timew;
        let dx = tx - 12.0 - datew;
        g.txt(&date, dx, 14.0, 13.0, [235, 235, 235], al, 200.0);
        g.txtb(&time, tx, 14.0, 13.0, [255, 255, 255], al, 80.0);
        let done = self.todos.iter().filter(|t| t.0).count();
        let total = self.todos.len();
        let frac = if total > 0 { done as f32 / total as f32 } else { 0.0 };
        g.ring(dx - 16.0, 23.0, 8.0, frac, 3.0, &paint(255, 255, 255, 0.16 * al), &acc(al));

        let (rx, rw, y0, max) = rows_geom(self.editing);
        if self.editing {
            let field = rrect(LX + ox, 40.0, CW, 30.0, 10.0);
            g.glass_panel(&field, al);
            g.stroke(&field, &acc(0.55 * al), 1.2);
            let shown: &str = if self.input.is_empty() { "Type a task, Enter to save, Esc to cancel" } else { &self.input };
            let col = if self.input.is_empty() { [120, 120, 125] } else { [255, 255, 255] };
            g.txt(shown, LX + 12.0 + ox, 40.0 + ty(30.0, 13.0), 13.0, col, al, CW - 30.0);
            if self.caret_on {
                let cw = if self.input.is_empty() { 0.0 } else { g.width(&self.input, 13.0).min(CW - 30.0) };
                g.rect(LX + 12.0 + ox + cw + 1.0, 46.0, 1.5, 18.0, &paint(255, 255, 255, al));
            }
        } else {
            g.txtb(&format!("{}/{}", done, total), x, 36.0, 42.0, [255, 255, 255], al, 190.0);
            g.txt("Tasks done today", x, 92.0, 12.0, [150, 150, 155], al, 190.0);
            self.pill(g, x, 116.0, "Add Task", false, false, al);
            self.pill(g, x + 98.0, 116.0, if self.edit_mode { "Done" } else { "Edit Task" }, true, self.edit_mode, al);
            if self.todos.is_empty() {
                g.txt("No tasks yet", rx + ox + 8.0, 56.0, 13.0, [140, 140, 145], al, 180.0);
            }
        }
        let order = self.todo_order();
        for (vi, &ri) in order.iter().skip(self.todo_scroll).take(max).enumerate() {
            self.todo_row(g, rx + ox, y0 + vi as f32 * ROW_P, rw, ri, al);
        }
        let n = order.len();
        if n > max {
            let th = max as f32 * ROW_P - (ROW_P - ROW_H);
            let thumb = (th * max as f32 / n as f32).max(14.0);
            let ty0 = y0 + (th - thumb) * self.todo_scroll as f32 / (n - max) as f32;
            g.fill(&rrect(RX - 4.0 + ox, y0, 3.0, th, 1.5), &paint(255, 255, 255, 0.10 * al));
            g.fill(&rrect(RX - 4.0 + ox, ty0, 3.0, thumb, 1.5), &paint(255, 255, 255, 0.55 * al));
        }
    }
}