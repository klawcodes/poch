//! Rendering dasar: font/teks, kanvas Gfx, bentuk & helper gambar.

use super::*;

pub(crate) fn pixmap_from_rgba(rgba: &[u8], size: u32) -> Option<Pixmap> {
    let mut pm = Pixmap::new(size, size)?;
    for (d, s) in pm.data_mut().chunks_exact_mut(4).zip(rgba.chunks_exact(4)) {
        let al = s[3] as u32;
        d[0] = (s[0] as u32 * al / 255) as u8;
        d[1] = (s[1] as u32 * al / 255) as u8;
        d[2] = (s[2] as u32 * al / 255) as u8;
        d[3] = s[3];
    }
    Some(pm)
}

static FONT_REG: &[u8] = include_bytes!("../assets/InstrumentSans-Regular.ttf");
static FONT_REG_EXT: &[u8] = include_bytes!("../assets/InstrumentSans-Ext-Regular.ttf");
static FONT_BOLD: &[u8] = include_bytes!("../assets/InstrumentSans-Bold.ttf");
static FONT_BOLD_EXT: &[u8] = include_bytes!("../assets/InstrumentSans-Ext-Bold.ttf");

const ASC: f32 = 1.079;
const GLYPH_CACHE_MAX: usize = 400;

pub(crate) type GlyphKey = (char, u32, bool);

pub(crate) struct Text {
    pub(crate) reg: Vec<fontdue::Font>,
    pub(crate) bold: Vec<fontdue::Font>,
    pub(crate) fallback_tried: bool,
    pub(crate) cache: HashMap<GlyphKey, (fontdue::Metrics, Vec<u8>)>,
}

fn load_font(data: &[u8]) -> Option<fontdue::Font> {
    fontdue::Font::from_bytes(data, fontdue::FontSettings::default()).ok()
}

fn glyph_of<'a>(
    cache: &'a mut HashMap<GlyphKey, (fontdue::Metrics, Vec<u8>)>,
    chain: &[fontdue::Font],
    ch: char,
    size: f32,
    bold: bool,
) -> &'a (fontdue::Metrics, Vec<u8>) {
    let key = (ch, size as u32, bold);
    if !cache.contains_key(&key) && cache.len() >= GLYPH_CACHE_MAX {
        cache.clear();
    }
    cache.entry(key).or_insert_with(|| {
        let f = chain.iter().find(|f| f.lookup_glyph_index(ch) != 0).unwrap_or(&chain[0]);
        f.rasterize(ch, size)
    })
}

impl Text {
    pub(crate) fn load() -> Option<Text> {
        let mut reg = vec![load_font(FONT_REG)?];
        let mut bold = vec![load_font(FONT_BOLD)?];
        reg.extend(load_font(FONT_REG_EXT));
        bold.extend(load_font(FONT_BOLD_EXT));
        Some(Text { reg, bold, fallback_tried: false, cache: HashMap::new() })
    }

    fn ensure_fallback(&mut self, s: &str) {
        if self.fallback_tried {
            return;
        }
        let missing = s.chars().any(|c| !c.is_control() && self.reg.iter().all(|f| f.lookup_glyph_index(c) == 0));
        if !missing {
            return;
        }
        self.fallback_tried = true;
        if let Some(f) = std::fs::read(r"C:\Windows\Fonts\segoeui.ttf").ok().and_then(|d| load_font(&d)) {
            self.reg.push(f);
        }
        if let Some(f) = std::fs::read(r"C:\Windows\Fonts\segoeuib.ttf").ok().and_then(|d| load_font(&d)) {
            self.bold.push(f);
        }
        self.cache.clear();
    }

    fn measure(&mut self, s: &str, size: f32, bold: bool) -> f32 {
        self.ensure_fallback(s);
        let chain: &[fontdue::Font] = if bold { &self.bold } else { &self.reg };
        let mut total = 0.0;
        for ch in s.chars() {
            let (m, _) = glyph_of(&mut self.cache, chain, ch, size, bold);
            total += m.advance_width;
        }
        total
    }

    pub(crate) fn draw(
        &mut self,
        pm: &mut Pixmap,
        s: &str,
        x: f32,
        y: f32,
        size: f32,
        col: [u8; 3],
        alpha: f32,
        max_w: f32,
        bold: bool,
        clip: Option<(f32, f32, f32)>,
    ) {
        if alpha <= 0.02 {
            return;
        }
        self.ensure_fallback(s);
        let (pw, ph) = (pm.width() as i32, pm.height() as i32);
        let chain: &[fontdue::Font] = if bold { &self.bold } else { &self.reg };
        let asc = size * ASC;
        let mut pen = 0.0f32;
        let data = pm.data_mut();
        for ch in s.chars() {
            let (m, bm) = glyph_of(&mut self.cache, chain, ch, size, bold);
            if pen + m.advance_width > max_w {
                break;
            }
            let gx = (x + pen).round() as i32 + m.xmin;
            let gy = (y + asc).round() as i32 - m.height as i32 - m.ymin;
            let skip = match clip {
                Some((cl, cr, _)) => ((gx + m.width as i32) as f32) < cl || (gx as f32) > cr,
                None => false,
            };
            if !skip {
                for row in 0..m.height {
                    for colx in 0..m.width {
                        let cov = bm[row * m.width + colx];
                        if cov == 0 {
                            continue;
                        }
                        let (px, py) = (gx + colx as i32, gy + row as i32);
                        if px < 0 || py < 0 || px >= pw || py >= ph {
                            continue;
                        }
                        let mut a = cov as f32 / 255.0 * alpha;
                        if let Some((cl, cr, fd)) = clip {
                            let fx = px as f32 + 0.5;
                            if fx < cl || fx >= cr {
                                continue;
                            }
                            if fd > 0.0 {
                                a *= ((fx - cl) / fd).clamp(0.0, 1.0) * ((cr - fx) / fd).clamp(0.0, 1.0);
                            }
                        }
                        let i = ((py * pw + px) * 4) as usize;
                        for c in 0..3 {
                            data[i + c] = (col[c] as f32 * a + data[i + c] as f32 * (1.0 - a)) as u8;
                        }
                        data[i + 3] = (255.0 * a + data[i + 3] as f32 * (1.0 - a)) as u8;
                    }
                }
            }
            pen += m.advance_width;
        }
    }
}

/// Pixel box (x0, y0, x1, y1), end-exclusive. Used so we only clear / mask / copy what changed.
pub(crate) type Box4 = (usize, usize, usize, usize);

pub(crate) fn union_box(a: Box4, b: Box4) -> Box4 {
    (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3))
}

pub(crate) struct Gfx {
    pub(crate) pm: Pixmap,
    pub(crate) text: Option<Text>,
    pub(crate) mask: Option<Mask>,
    /// Region touched by the previous frame (so it gets cleared on the next one).
    pub(crate) prev_box: Box4,
    /// Region that must be copied to the window bitmap for the current frame.
    pub(crate) dirty: Box4,
}

impl Gfx {
    /// Frosted glass body: black tint over the blurred backdrop, top sheen, diagonal highlight, rim light.
    /// `tint` is the overlay opacity; `al` scales the highlights so the idle pill stays plain black.
    pub(crate) fn glass_body(&mut self, path: &Path, tint: f32, al: f32) {
        use crate::config::GLASS_SHEEN;
        self.fill(path, &paint(0, 0, 0, tint));
        let b = path.bounds();
        if al <= 0.01 || b.height() < 12.0 {
            return;
        }
        let a8 = |v: f32| (v * al * 255.0).clamp(0.0, 255.0) as u8;
        let (l, t, w, h) = (b.left(), b.top(), b.width(), b.height());
        if let Some(p) = white_gradient((0.0, t), (0.0, t + h * 0.6), &[(0.0, a8(GLASS_SHEEN)), (1.0, 0)]) {
            self.fill(path, &p);
        }
        if let Some(p) = white_gradient((l, t), (l + w * 0.6, t + h), &[(0.0, a8(0.10)), (0.55, 0)]) {
            self.fill(path, &p);
        }
        if let Some(p) = white_gradient((0.0, t), (0.0, t + h), &[(0.0, a8(0.42)), (0.5, a8(0.10)), (1.0, a8(0.24))]) {
            // The top 3 px are skipped so no bright line sits on the screen edge.
            let mask = Mask::new(W as u32, H as u32).map(|mut m| {
                if let Some(r) = Rect::from_xywh(0.0, 3.0, W as f32, H as f32) {
                    m.fill_path(&PathBuilder::from_rect(r), FillRule::Winding, false, Transform::identity());
                }
                m
            });
            let st = Stroke { width: 1.2, ..Default::default() };
            self.pm.stroke_path(path, &p, &st, Transform::identity(), mask.as_ref());
        }
    }

    /// Nested panel: faint white lift with a hairline that is brighter on top.
    pub(crate) fn glass_panel(&mut self, path: &Path, al: f32) {
        use crate::config::GLASS_PANEL;
        self.fill(path, &paint(255, 255, 255, GLASS_PANEL * al));
        let b = path.bounds();
        let a8 = |v: f32| (v * al * 255.0).clamp(0.0, 255.0) as u8;
        if let Some(p) = white_gradient((0.0, b.top()), (0.0, b.bottom()), &[(0.0, a8(0.22)), (1.0, a8(0.05))]) {
            let st = Stroke { width: 1.0, ..Default::default() };
            self.pm.stroke_path(path, &p, &st, Transform::identity(), None);
        }
    }

    pub(crate) fn clear_box(&mut self, b: Box4) {
        let pw = self.pm.width() as usize;
        let ph = self.pm.height() as usize;
        let (x0, y0, x1, y1) = (b.0, b.1, b.2.min(pw), b.3.min(ph));
        if x1 <= x0 {
            return;
        }
        let data = self.pm.data_mut();
        for y in y0..y1 {
            data[(y * pw + x0) * 4..(y * pw + x1) * 4].fill(0);
        }
    }

    pub(crate) fn fill(&mut self, path: &Path, p: &Paint) {
        self.pm.fill_path(path, p, FillRule::Winding, Transform::identity(), None);
    }
    pub(crate) fn stroke(&mut self, path: &Path, p: &Paint, w: f32) {
        let st = Stroke { width: w, ..Default::default() };
        self.pm.stroke_path(path, p, &st, Transform::identity(), None);
    }
    pub(crate) fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, p: &Paint) {
        if let Some(r) = Rect::from_xywh(x, y, w, h) {
            self.pm.fill_rect(r, p, Transform::identity(), None);
        }
    }
    pub(crate) fn circle(&mut self, x: f32, y: f32, r: f32, p: &Paint) {
        if let Some(path) = PathBuilder::from_circle(x, y, r) {
            self.fill(&path, p);
        }
    }
    pub(crate) fn poly(&mut self, pts: &[(f32, f32)], p: &Paint) {
        let mut pb = PathBuilder::new();
        pb.move_to(pts[0].0, pts[0].1);
        for q in &pts[1..] {
            pb.line_to(q.0, q.1);
        }
        pb.close();
        if let Some(path) = pb.finish() {
            self.fill(&path, p);
        }
    }
    pub(crate) fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, wd: f32, p: &Paint) {
        let (dx, dy) = (x2 - x1, y2 - y1);
        let len = (dx * dx + dy * dy).sqrt().max(0.001);
        let (nx, ny) = (-dy / len * wd / 2.0, dx / len * wd / 2.0);
        self.poly(&[(x1 + nx, y1 + ny), (x2 + nx, y2 + ny), (x2 - nx, y2 - ny), (x1 - nx, y1 - ny)], p);
        self.circle(x1, y1, wd / 2.0, p);
        self.circle(x2, y2, wd / 2.0, p);
    }
    fn txt_ex(&mut self, s: &str, x: f32, y: f32, size: f32, col: [u8; 3], al: f32, max_w: f32, bold: bool) {
        if let Some(t) = self.text.as_mut() {
            t.draw(&mut self.pm, s, x, y, size, col, al, max_w, bold, None);
        }
    }
    pub(crate) fn txt_clip(&mut self, s: &str, x: f32, y: f32, size: f32, col: [u8; 3], al: f32, cl: f32, cr: f32, fade: f32) {
        if let Some(t) = self.text.as_mut() {
            t.draw(&mut self.pm, s, x, y, size, col, al, f32::INFINITY, false, Some((cl, cr, fade)));
        }
    }
    pub(crate) fn txt(&mut self, s: &str, x: f32, y: f32, size: f32, col: [u8; 3], al: f32, max_w: f32) {
        self.txt_ex(s, x, y, size, col, al, max_w, false);
    }
    pub(crate) fn txtb(&mut self, s: &str, x: f32, y: f32, size: f32, col: [u8; 3], al: f32, max_w: f32) {
        self.txt_ex(s, x, y, size, col, al, max_w, true);
    }
    pub(crate) fn width_ex(&mut self, s: &str, size: f32, bold: bool) -> f32 {
        self.text.as_mut().map(|t| t.measure(s, size, bold)).unwrap_or(0.0)
    }
    pub(crate) fn width(&mut self, s: &str, size: f32) -> f32 {
        self.width_ex(s, size, false)
    }
    pub(crate) fn txt_c(&mut self, s: &str, cx: f32, y: f32, size: f32, col: [u8; 3], al: f32, max_w: f32, bold: bool) {
        let tw = self.width_ex(s, size, bold).min(max_w);
        self.txt_ex(s, cx - tw / 2.0, y, size, col, al, max_w, bold);
    }
    pub(crate) fn ring(&mut self, cx: f32, cy: f32, r: f32, frac: f32, wd: f32, track: &Paint, fg: &Paint) {
        if let Some(path) = PathBuilder::from_circle(cx, cy, r) {
            let st = Stroke { width: wd, ..Default::default() };
            self.pm.stroke_path(&path, track, &st, Transform::identity(), None);
        }
        if frac > 0.001 {
            let steps = ((frac * 64.0).ceil() as i32).max(2);
            let mut pb = PathBuilder::new();
            for s in 0..=steps {
                let ang = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * frac.min(1.0) * s as f32 / steps as f32;
                let (px, py) = (cx + r * ang.cos(), cy + r * ang.sin());
                if s == 0 {
                    pb.move_to(px, py);
                } else {
                    pb.line_to(px, py);
                }
            }
            if let Some(path) = pb.finish() {
                let st = Stroke { width: wd, line_cap: LineCap::Round, ..Default::default() };
                self.pm.stroke_path(&path, fg, &st, Transform::identity(), None);
            }
        }
    }
}

fn white_gradient(p0: (f32, f32), p1: (f32, f32), stops: &[(f32, u8)]) -> Option<Paint<'static>> {
    let st = stops.iter().map(|&(o, a)| GradientStop::new(o, Color::from_rgba8(255, 255, 255, a))).collect();
    let shader = LinearGradient::new(Point::from_xy(p0.0, p0.1), Point::from_xy(p1.0, p1.1), st, SpreadMode::Pad, Transform::identity())?;
    let mut p = Paint::default();
    p.shader = shader;
    p.anti_alias = true;
    Some(p)
}

pub(crate) fn dummy_path() -> Path {
    PathBuilder::from_rect(Rect::from_xywh(0.0, 0.0, 1.0, 1.0).unwrap())
}

pub(crate) fn rrect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Path {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.quad_to(x + w, y, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    pb.finish().unwrap_or_else(dummy_path)
}

pub(crate) fn notch_path(w: f32, h: f32) -> Path {
    notch_path_w(w, h, 0.0)
}

/// Bentuk notch yang sama, tapi tepi bawah "melorot" sebesar `dip` px (pulse droplet).
/// `dip = 0` identik dengan bentuk normal; `dip < 0` berarti tepi memantul ke atas.
pub(crate) fn notch_path_w(w: f32, h: f32, dip: f32) -> Path {
    let dip = dip.min(H as f32 - 1.0 - h); // jangan keluar dari kanvas
    let ear = (h * 0.35).min(10.0);
    let rb = (h * 0.5).min(22.0);
    let l = CX - w / 2.0;
    let r = CX + w / 2.0;
    let mut pb = PathBuilder::new();
    pb.move_to(l - ear, 0.0);
    pb.line_to(r + ear, 0.0);
    pb.quad_to(r, 0.0, r, ear);
    pb.line_to(r, h - rb);
    pb.quad_to(r, h, r - rb, h);
    if dip.abs() < 0.2 {
        pb.line_to(l + rb, h);
    } else {
        // Tonjolan tetesan di tengah tepi bawah; makin turun makin ramping (leher), memantul melebar.
        let a = r - rb;
        let b = l + rb;
        let base = ((r - l) * 0.30).min(110.0).max(24.0);
        let bw = (base * (1.0 - 0.3 * (dip / DROP_AMP).clamp(-0.5, 1.0))).min((a - b) / 2.0 - 1.0).max(4.0);
        pb.line_to(CX + bw, h);
        pb.cubic_to(CX + bw * 0.55, h, CX + bw * 0.50, h + dip, CX, h + dip);
        pb.cubic_to(CX - bw * 0.50, h + dip, CX - bw * 0.55, h, CX - bw, h);
        pb.line_to(b, h);
    }
    pb.quad_to(l, h, l, h - rb);
    pb.line_to(l, ear);
    pb.quad_to(l, 0.0, l - ear, 0.0);
    pb.close();
    pb.finish().unwrap_or_else(dummy_path)
}

pub(crate) fn paint(r: u8, g: u8, b: u8, a: f32) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(r, g, b, (a.clamp(0.0, 1.0) * 255.0) as u8);
    p.anti_alias = true;
    p
}

pub(crate) fn acc(a: f32) -> Paint<'static> {
    paint(ACCENT[0], ACCENT[1], ACCENT[2], a)
}

pub(crate) fn in_rect(x: f32, y: f32, rx: f32, ry: f32, rw: f32, rh: f32) -> bool {
    x >= rx && x <= rx + rw && y >= ry && y <= ry + rh
}

pub(crate) fn ty(h: f32, size: f32) -> f32 {
    (h - size * 1.33) / 2.0
}

/// Greedy word wrap (breaks long words by character). '\n' always starts a new line.
pub(crate) fn wrap_lines(g: &mut Gfx, s: &str, size: f32, maxw: f32) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for para in s.split('\n') {
        let mut line = String::new();
        let mut lw = 0.0f32;
        let mut last_sp: Option<usize> = None;
        for ch in para.chars() {
            let mut b = [0u8; 4];
            let cw = g.width(ch.encode_utf8(&mut b), size);
            if lw + cw > maxw && !line.is_empty() {
                match last_sp {
                    Some(sp) if ch != ' ' => {
                        let rest = line[sp + 1..].to_string();
                        line.truncate(sp);
                        out.push(std::mem::replace(&mut line, rest));
                        lw = g.width(&line, size);
                    }
                    _ => {
                        out.push(std::mem::take(&mut line));
                        lw = 0.0;
                        if ch == ' ' {
                            last_sp = None;
                            continue;
                        }
                    }
                }
                last_sp = None;
            }
            if ch == ' ' {
                last_sp = Some(line.len());
            }
            line.push(ch);
            lw += cw;
        }
        out.push(line);
    }
    out
}

/// Cuts `s` so that `s + "..."` fits in `maxw`.
pub(crate) fn ellipsize(g: &mut Gfx, s: &str, size: f32, maxw: f32) -> String {
    let mut t = s.to_string();
    while !t.is_empty() && g.width(&format!("{}\u{2026}", t), size) > maxw {
        t.pop();
    }
    t.push('\u{2026}');
    t
}