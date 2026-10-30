//! Fitur Scrap: notepad mini. Scrollable, kursor + seleksi, maks 3.333 karakter, slide Scrap.

use super::*;

use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::Foundation::GlobalFree;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

const NOTE_NAME: &str = "Scrap";

pub(crate) const NOTE_Y: f32 = 38.0;
pub(crate) const NOTE_H: f32 = 100.0;
const NOTE_SIZE: f32 = 13.0;
const NOTE_LINE: f32 = 17.0;
const NOTE_LINES: usize = 5; // visible lines; the notch never grows, the text scrolls instead
const NOTE_TOP: f32 = 12.0; // first line offset inside the card
const NOTE_W: f32 = CW - 28.0;
pub(crate) const NOTE_MAX_CHARS: usize = 3333;

const CF_UNICODETEXT: u32 = 13;
const WHEEL_STEP: i32 = 40; // wheel delta per line (one 120 notch = 3 lines, trackpads accumulate)

// Virtual-key codes handled in WM_KEYDOWN.
const K_PGUP: u32 = 0x21;
const K_PGDN: u32 = 0x22;
const K_END: u32 = 0x23;
const K_HOME: u32 = 0x24;
const K_LEFT: u32 = 0x25;
const K_UP: u32 = 0x26;
const K_RIGHT: u32 = 0x27;
const K_DOWN: u32 = 0x28;
const K_DELETE: u32 = 0x2E;

/// Caret / selection / scroll state of the note editor (char indices, not bytes).
#[derive(Default)]
pub(crate) struct NoteView {
    pub(crate) cur: usize,
    pub(crate) anchor: usize,
    pub(crate) scroll: usize,
    pub(crate) sel_drag: bool,
    wheel: i32,
}

pub(crate) fn load_note() -> String {
    let s = std::fs::read_to_string(data_path("notepad.txt")).unwrap_or_default();
    s.replace("\r\n", "\n").chars().take(NOTE_MAX_CHARS).collect()
}

pub(crate) fn save_note(s: &str) {
    write_data("notepad.txt", s.to_string());
}

fn char_w(g: &mut Gfx, ch: char) -> f32 {
    let mut b = [0u8; 4];
    g.width(ch.encode_utf8(&mut b), NOTE_SIZE)
}

fn span_w(g: &mut Gfx, chars: &[char]) -> f32 {
    chars.iter().map(|&c| char_w(g, c)).sum()
}

/// Wrapped lines as char ranges `[start, end)`. Unlike `wrap_lines` no character is dropped
/// (spaces stay at the end of the line they wrap on, '\n' sits between two ranges), so
/// caret / selection / hit-testing map exactly onto the text.
fn layout(g: &mut Gfx, s: &str) -> Vec<(usize, usize)> {
    let chars: Vec<char> = s.chars().collect();
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut base = 0usize;
    for para in chars.split(|&c| c == '\n') {
        let n = para.len();
        let ws: Vec<f32> = para.iter().map(|&c| char_w(g, c)).collect();
        let mut start = 0usize;
        let mut w = 0.0f32;
        let mut brk: Option<usize> = None; // index just after the last space
        let mut i = 0usize;
        while i < n {
            let c = para[i];
            if c != ' ' && w + ws[i] > NOTE_W && i > start {
                let b = match brk {
                    Some(b) if b > start => b,
                    _ => i,
                };
                out.push((base + start, base + b));
                start = b;
                w = ws[start..i].iter().sum();
                brk = None;
                continue;
            }
            w += ws[i];
            if c == ' ' {
                brk = Some(i + 1);
            }
            i += 1;
        }
        out.push((base + start, base + n));
        base += n + 1;
    }
    out
}

/// Index of the visual line that holds the caret.
fn line_of(lines: &[(usize, usize)], cur: usize) -> usize {
    lines.iter().rposition(|&(s, _)| s <= cur).unwrap_or(0)
}

/// Caret position for "End" / clamping on line `li` (stays on this line even when it soft-wraps).
fn line_end(lines: &[(usize, usize)], li: usize) -> usize {
    let (s, e) = lines[li];
    if li + 1 < lines.len() && lines[li + 1].0 == e && e > s {
        e - 1
    } else {
        e
    }
}

/// Char index on line `li` closest to pixel offset `px` (relative to the text origin).
fn index_at(g: &mut Gfx, chars: &[char], lines: &[(usize, usize)], li: usize, px: f32) -> usize {
    let s = lines[li].0;
    let e = line_end(lines, li);
    let mut acc = 0.0f32;
    let mut idx = s;
    while idx < e {
        let w = char_w(g, chars[idx]);
        if acc + w / 2.0 > px {
            break;
        }
        acc += w;
        idx += 1;
    }
    idx
}

fn word_left(v: &[char], mut i: usize) -> usize {
    while i > 0 && v[i - 1].is_whitespace() {
        i -= 1;
    }
    while i > 0 && !v[i - 1].is_whitespace() {
        i -= 1;
    }
    i
}

fn word_right(v: &[char], mut i: usize) -> usize {
    while i < v.len() && !v[i].is_whitespace() {
        i += 1;
    }
    while i < v.len() && v[i].is_whitespace() {
        i += 1;
    }
    i
}

/// Ctrl+Backspace: trailing spaces, then either one newline or the previous word.
fn word_back(v: &[char], from: usize) -> usize {
    let mut j = from;
    while j > 0 && v[j - 1] == ' ' {
        j -= 1;
    }
    if j > 0 && v[j - 1] == '\n' {
        j - 1
    } else {
        while j > 0 && v[j - 1] != ' ' && v[j - 1] != '\n' {
            j -= 1;
        }
        j
    }
}

fn fmt_num(n: usize) -> String {
    let s = n.to_string();
    let mut o = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            o.push(',');
        }
        o.push(c);
    }
    o
}

unsafe fn clip_set(hwnd: HWND, text: &str) {
    if OpenClipboard(hwnd).is_err() {
        return;
    }
    let _ = EmptyClipboard();
    let wide: Vec<u16> = text.replace('\n', "\r\n").encode_utf16().chain(std::iter::once(0)).collect();
    if let Ok(h) = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2) {
        let p = GlobalLock(h) as *mut u16;
        if p.is_null() {
            let _ = GlobalFree(h);
        } else {
            std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
            let _ = GlobalUnlock(h);
            if SetClipboardData(CF_UNICODETEXT, HANDLE(h.0)).is_err() {
                let _ = GlobalFree(h);
            }
        }
    }
    let _ = CloseClipboard();
}

unsafe fn clip_get(hwnd: HWND) -> Option<String> {
    if OpenClipboard(hwnd).is_err() {
        return None;
    }
    let mut out: Option<String> = None;
    if let Ok(h) = GetClipboardData(CF_UNICODETEXT) {
        let hg = HGLOBAL(h.0);
        let p = GlobalLock(hg) as *const u16;
        if !p.is_null() {
            let mut len = 0usize;
            while len < (1 << 20) && *p.add(len) != 0 {
                len += 1;
            }
            out = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, len)));
            let _ = GlobalUnlock(hg);
        }
    }
    let _ = CloseClipboard();
    out
}

impl App {
    /// True when `s` respects the character limit.
    pub(crate) fn note_fits(&mut self, s: &str) -> bool {
        s.chars().count() <= NOTE_MAX_CHARS
    }

    /// Keep caret/anchor valid (the note can be changed from elsewhere, e.g. "Clear").
    fn clamp_nv(&mut self) {
        let n = self.note.chars().count();
        self.nv.cur = self.nv.cur.min(n);
        self.nv.anchor = self.nv.anchor.min(n);
    }

    fn sel_range(&self) -> (usize, usize) {
        (self.nv.cur.min(self.nv.anchor), self.nv.cur.max(self.nv.anchor))
    }

    fn sel_text(&mut self) -> Option<String> {
        self.clamp_nv();
        let (lo, hi) = self.sel_range();
        if hi > lo {
            Some(self.note.chars().skip(lo).take(hi - lo).collect())
        } else {
            None
        }
    }

    pub(crate) unsafe fn begin_note(&mut self) {
        self.editing = true;
        self.note_mode = true;
        self.caret_on = true;
        self.edit_start = Instant::now();
        let n = self.note.chars().count();
        self.nv.cur = n;
        self.nv.anchor = n;
        self.nv.sel_drag = false;
        self.note_ensure_visible();
        self.prev_fg = GetForegroundWindow().0 as isize;
        let hwnd = HWND(self.hwnd as *mut c_void);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(hwnd);
    }

    /// Scroll so the caret line is inside the visible window.
    pub(crate) fn note_ensure_visible(&mut self) {
        let cur = self.nv.cur.min(self.note.chars().count());
        let Some(g) = self.gfx.as_mut() else { return; };
        let lines = layout(g, &self.note);
        let li = line_of(&lines, cur);
        let max = lines.len().saturating_sub(NOTE_LINES);
        let mut s = self.nv.scroll.min(max);
        if li < s {
            s = li;
        } else if li >= s + NOTE_LINES {
            s = li + 1 - NOTE_LINES;
        }
        self.nv.scroll = s.min(max);
    }

    /// Common tail of every editing action: keep caret visible, restart blink, repaint.
    unsafe fn note_refresh(&mut self) {
        self.note_ensure_visible();
        self.edit_start = Instant::now();
        self.caret_on = true;
        self.last = Instant::now();
        present(self);
        schedule(self);
    }

    /// Insert text at the caret (replacing the selection). Input beyond the limit is cut off.
    fn note_insert(&mut self, text: &str) -> bool {
        let mut ins: Vec<char> = Vec::new();
        let mut prev_cr = false;
        for ch in text.chars() {
            match ch {
                '\r' => {
                    ins.push('\n');
                    prev_cr = true;
                    continue;
                }
                '\n' => {
                    if !prev_cr {
                        ins.push('\n');
                    }
                }
                '\t' => ins.push(' '),
                c if (c as u32) >= 32 && (c as u32) != 127 => ins.push(c),
                _ => {}
            }
            prev_cr = false;
        }
        if ins.is_empty() {
            return false;
        }
        self.clamp_nv();
        let (lo, hi) = self.sel_range();
        let mut v: Vec<char> = self.note.chars().collect();
        let room = NOTE_MAX_CHARS.saturating_sub(v.len() - (hi - lo));
        if room == 0 {
            return false;
        }
        ins.truncate(room); // pasted text is truncated to the remaining allowance
        let p = lo + ins.len();
        v.splice(lo..hi, ins);
        self.note = v.into_iter().collect();
        self.nv.cur = p;
        self.nv.anchor = p;
        true
    }

    fn note_delete(&mut self, forward: bool, word: bool) -> bool {
        self.clamp_nv();
        let (lo, hi) = self.sel_range();
        let mut v: Vec<char> = self.note.chars().collect();
        let (a, b) = if hi > lo {
            (lo, hi)
        } else if forward {
            if hi >= v.len() {
                return false;
            }
            (hi, hi + 1)
        } else {
            if lo == 0 {
                return false;
            }
            (if word { word_back(&v, lo) } else { lo - 1 }, lo)
        };
        v.drain(a..b);
        self.note = v.into_iter().collect();
        self.nv.cur = a;
        self.nv.anchor = a;
        true
    }

    /// WM_CHAR while the note is being edited.
    pub(crate) unsafe fn note_char(&mut self, c: u32) {
        let hwnd = HWND(self.hwnd as *mut c_void);
        match c {
            27 => self.finish_edit(false, true),
            1 => {
                // Ctrl+A
                self.nv.anchor = 0;
                self.nv.cur = self.note.chars().count();
            }
            3 => {
                // Ctrl+C
                if let Some(t) = self.sel_text() {
                    clip_set(hwnd, &t);
                }
            }
            24 => {
                // Ctrl+X
                if let Some(t) = self.sel_text() {
                    clip_set(hwnd, &t);
                    self.note_delete(false, false);
                }
            }
            22 => {
                // Ctrl+V
                if let Some(t) = clip_get(hwnd) {
                    self.note_insert(&t);
                }
            }
            13 => {
                self.note_insert("\n");
            }
            8 => {
                self.note_delete(false, false);
            }
            127 => {
                self.note_delete(false, true); // Ctrl+Backspace
            }
            c if c >= 32 => {
                if let Some(ch) = char::from_u32(c) {
                    let mut b = [0u8; 4];
                    self.note_insert(ch.encode_utf8(&mut b));
                }
            }
            _ => {}
        }
        self.note_refresh();
    }

    /// Target caret index for Home/End/Up/Down/PageUp/PageDown.
    fn note_nav(&mut self, vk: u32, ctrl: bool, v: &[char]) -> usize {
        let n = v.len();
        let cur = self.nv.cur.min(n);
        let Some(g) = self.gfx.as_mut() else { return cur; };
        let lines = layout(g, &self.note);
        let li = line_of(&lines, cur);
        match vk {
            K_HOME => {
                if ctrl {
                    0
                } else {
                    lines[li].0
                }
            }
            K_END => {
                if ctrl {
                    n
                } else {
                    line_end(&lines, li)
                }
            }
            _ => {
                let up = vk == K_UP || vk == K_PGUP;
                let step = if vk == K_UP || vk == K_DOWN { 1 } else { NOTE_LINES };
                if up && li == 0 {
                    return 0;
                }
                if !up && li + 1 >= lines.len() {
                    return n;
                }
                let tl = if up { li.saturating_sub(step) } else { (li + step).min(lines.len() - 1) };
                let px = span_w(g, &v[lines[li].0..cur]);
                index_at(g, v, &lines, tl, px)
            }
        }
    }

    /// WM_KEYDOWN while editing the note. Returns true when the key was consumed.
    pub(crate) unsafe fn note_key(&mut self, vk: u32, shift: bool, ctrl: bool) -> bool {
        self.clamp_nv();
        if vk == K_DELETE {
            self.note_delete(true, false);
            self.note_refresh();
            return true;
        }
        let v: Vec<char> = self.note.chars().collect();
        let n = v.len();
        let (lo, hi) = self.sel_range();
        let cur = self.nv.cur;
        let target = match vk {
            K_LEFT => {
                if !shift && hi > lo {
                    lo
                } else if ctrl {
                    word_left(&v, cur)
                } else {
                    cur.saturating_sub(1)
                }
            }
            K_RIGHT => {
                if !shift && hi > lo {
                    hi
                } else if ctrl {
                    word_right(&v, cur)
                } else {
                    (cur + 1).min(n)
                }
            }
            K_UP | K_DOWN | K_PGUP | K_PGDN | K_HOME | K_END => self.note_nav(vk, ctrl, &v),
            _ => return false,
        };
        self.nv.cur = target;
        if !shift {
            self.nv.anchor = target;
        }
        self.note_refresh();
        true
    }

    /// Char index under a point (client coordinates).
    fn note_hit(&mut self, x: f32, y: f32) -> usize {
        let cur = self.nv.cur;
        let chars: Vec<char> = self.note.chars().collect();
        let Some(g) = self.gfx.as_mut() else { return cur; };
        let lines = layout(g, &self.note);
        let max = lines.len().saturating_sub(NOTE_LINES);
        let scroll = self.nv.scroll.min(max) as i32;
        let rel = ((y - (NOTE_Y + NOTE_TOP)) / NOTE_LINE).floor() as i32;
        let li = (scroll + rel).clamp(0, lines.len() as i32 - 1) as usize;
        index_at(g, &chars, &lines, li, x - (LX + 14.0))
    }

    /// Mouse press inside the note card while editing: place caret, start drag-selection.
    pub(crate) unsafe fn note_mouse_down(&mut self, x: f32, y: f32, shift: bool) -> bool {
        if !(self.ready() && self.editing && self.note_mode && self.cur() == 5 && in_rect(x, y, LX, NOTE_Y, CW, NOTE_H)) {
            return false;
        }
        self.clamp_nv();
        let idx = self.note_hit(x, y);
        self.nv.cur = idx;
        if !shift {
            self.nv.anchor = idx;
        }
        self.nv.sel_drag = true;
        self.note_refresh();
        true
    }

    pub(crate) unsafe fn note_mouse_drag(&mut self, x: f32, y: f32) {
        let idx = self.note_hit(x, y);
        if idx != self.nv.cur {
            self.nv.cur = idx;
            self.note_refresh();
        }
    }

    /// Mouse wheel / trackpad over the Scrap slide. Returns false when the text fits (no scrolling needed).
    pub(crate) fn note_wheel(&mut self, d: i32) -> bool {
        let Some(g) = self.gfx.as_mut() else { return false; };
        let total = layout(g, &self.note).len();
        if total <= NOTE_LINES {
            self.nv.wheel = 0;
            return false;
        }
        let max = total - NOTE_LINES;
        self.nv.wheel += d;
        let steps = self.nv.wheel / WHEEL_STEP;
        if steps != 0 {
            self.nv.wheel -= steps * WHEEL_STEP;
            let s = self.nv.scroll.min(max) as i32 - steps;
            self.nv.scroll = s.clamp(0, max as i32) as usize;
        }
        true
    }

    pub(crate) fn slide_note(&self, g: &mut Gfx, ox: f32, al: f32) {
        let x = LX + ox;
        g.txtb(NOTE_NAME, x, 12.0, 15.0, [255, 255, 255], al, 100.0);

        let editing = self.note_mode;
        let chars: Vec<char> = self.note.chars().collect();
        let n = chars.len();

        // Subtle live counter right after the title.
        if editing || n > 0 {
            let label = format!("{} / {}", fmt_num(n), fmt_num(NOTE_MAX_CHARS));
            let tw = g.width_ex(NOTE_NAME, 15.0, true);
            let col = if n >= NOTE_MAX_CHARS { ACCENT } else { [112, 112, 118] };
            g.txt(&label, x + tw + 10.0, 16.0, 11.0, col, al * if editing { 1.0 } else { 0.8 }, 140.0);
        }

        let (hint, hcol) = if editing {
            ("Esc to save", [150, 150, 155])
        } else if n > 0 {
            ("Clear", ACCENT)
        } else {
            ("", ACCENT)
        };
        if !hint.is_empty() {
            let hw = g.width(hint, 12.0);
            g.txt(hint, RX + ox - hw, 14.0, 12.0, hcol, al, 120.0);
        }

        // The accent edge marks the editing state; the tape strip stays as the one accent.
        let card = rrect(x, NOTE_Y, CW, NOTE_H, 12.0);
        g.glass_panel(&card, al);
        if editing {
            g.stroke(&card, &acc(0.55 * al), 1.2);
        }
        g.fill(&rrect(x + CW / 2.0 - 22.0, NOTE_Y - 3.0, 44.0, 8.0, 3.0), &acc(0.75 * al));

        let tx = x + 14.0;
        if n == 0 {
            if !editing {
                g.txt("Click to write a note", tx, NOTE_Y + NOTE_TOP, NOTE_SIZE, [140, 140, 145], al, NOTE_W);
            }
            if editing && self.caret_on {
                g.rect(tx, NOTE_Y + NOTE_TOP + 1.0, 1.5, 15.0, &paint(255, 255, 255, al));
            }
            return;
        }

        let lines = layout(g, &self.note);
        let total = lines.len();
        let max_scroll = total.saturating_sub(NOTE_LINES);
        let scroll = self.nv.scroll.min(max_scroll);
        let cur = self.nv.cur.min(n);
        let anc = self.nv.anchor.min(n);
        let (slo, shi) = (cur.min(anc), cur.max(anc));

        for li in scroll..(scroll + NOTE_LINES).min(total) {
            let (s, e) = lines[li];
            let y = NOTE_Y + NOTE_TOP + (li - scroll) as f32 * NOTE_LINE;

            // selection highlight behind the text
            if editing && shi > slo {
                let a = slo.max(s);
                let b = shi.min(e);
                let past_end = shi > e && slo <= e; // selection continues past this line's end
                if b > a || past_end {
                    let xa = span_w(g, &chars[s..a.min(e)]);
                    let mut xb = span_w(g, &chars[s..b.max(a).min(e)]);
                    if past_end {
                        xb += 5.0;
                    }
                    if xb > xa {
                        g.rect(tx + xa, y - 1.0, xb - xa, NOTE_LINE, &acc(0.55 * al));
                    }
                }
            }

            let text: String = chars[s..e].iter().collect();
            g.txt(&text, tx, y, NOTE_SIZE, [240, 240, 244], al, NOTE_W + 4.0);
        }

        if editing && self.caret_on && slo == shi {
            let li = line_of(&lines, cur);
            if li >= scroll && li < scroll + NOTE_LINES {
                let cx = span_w(g, &chars[lines[li].0..cur]);
                let y = NOTE_Y + NOTE_TOP + 1.0 + (li - scroll) as f32 * NOTE_LINE;
                g.rect(tx + cx + 0.5, y, 1.5, 15.0, &paint(255, 255, 255, al));
            }
        }

        // minimal scrollbar: a thin thumb on the card's right edge, only when the text overflows
        if total > NOTE_LINES {
            let top = NOTE_Y + 10.0;
            let th = NOTE_H - 20.0;
            let bh = (th * NOTE_LINES as f32 / total as f32).clamp(14.0, th);
            let pos = if max_scroll == 0 { 0.0 } else { scroll as f32 / max_scroll as f32 };
            let by = top + (th - bh) * pos;
            g.fill(&rrect(x + CW - 8.0, by, 2.5, bh, 1.25), &paint(255, 255, 255, (if editing { 0.34 } else { 0.22 }) * al));
        }
    }
}