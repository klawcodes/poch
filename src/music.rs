//! Fitur Music: GSMTC media thread, artwork, slide Now Playing.

use super::*;
use std::sync::atomic::{AtomicU8, Ordering};

const ART: u32 = 88;

pub(crate) const MARQUEE_SPEED: f32 = 38.0;
pub(crate) const MARQUEE_GAP: f32 = 34.0;
pub(crate) const MARQUEE_PAUSE: f32 = 1.2;

const ONLINE_ARTWORK: bool = true;

/// Phosphor Icons "music-notes" (fill), lisensi MIT, viewBox 256.
const NOTE_ICON: &str = "M212.92,17.71a7.89,7.89,0,0,0-6.86-1.46l-128,32A8,8,0,0,0,72,56V166.1A36,36,0,1,0,88,196V102.25l112-28V134.1A36,36,0,1,0,216,164V24A8,8,0,0,0,212.92,17.71Z";

fn note_icon_path() -> Option<&'static Path> {
    static ICON: std::sync::OnceLock<Option<Path>> = std::sync::OnceLock::new();
    ICON.get_or_init(|| {
        use svgtypes::{SimplePathSegment as Seg, SimplifyingPathParser};
        let mut pb = PathBuilder::new();
        for seg in SimplifyingPathParser::from(NOTE_ICON).flatten() {
            match seg {
                Seg::MoveTo { x, y } => pb.move_to(x as f32, y as f32),
                Seg::LineTo { x, y } => pb.line_to(x as f32, y as f32),
                Seg::Quadratic { x1, y1, x, y } => pb.quad_to(x1 as f32, y1 as f32, x as f32, y as f32),
                Seg::CurveTo { x1, y1, x2, y2, x, y } => {
                    pb.cubic_to(x1 as f32, y1 as f32, x2 as f32, y2 as f32, x as f32, y as f32)
                }
                Seg::ClosePath => pb.close(),
            }
        }
        pb.finish()
    })
    .as_ref()
}

pub(crate) const MX: f32 = LX + 104.0;
pub(crate) const CLOCK_W: f32 = 124.0; // lebar blok jam di kanan (rata kanan di RX)
pub(crate) const MW: f32 = RX - CLOCK_W - 6.0 - MX; // kolom teks/kontrol berhenti sebelum blok jam
pub(crate) const MCX: f32 = MX + MW / 2.0;
pub(crate) const MCY: f32 = 128.0;

#[derive(Clone, Default, PartialEq)]
pub(crate) struct Media {
    pub(crate) title: String,
    pub(crate) artist: String,
    pub(crate) playing: bool,
    pub(crate) pos: u32,
    pub(crate) dur: u32,
}

impl Media {
    pub(crate) fn key(&self) -> String {
        format!("{}|{}", self.title, self.artist)
    }
}

pub(crate) struct ArtData {
    pub(crate) key: String,
    pub(crate) rgba: Vec<u8>,
}

pub(crate) enum Cmd {
    Prev,
    Toggle,
    Next,
    Seek(f32),
}

fn fmt_time(s: u32) -> String {
    format!("{}:{:02}", s / 60, s % 60)
}

fn read_thumbnail(props: &MediaProps) -> Option<Vec<u8>> {
    let r = props.Thumbnail().ok()?;
    let s = r.OpenReadAsync().ok()?.get().ok()?;
    let size = s.Size().ok()? as u32;
    if size == 0 || size > 8_000_000 {
        return None;
    }
    let reader = DataReader::CreateDataReader(&s).ok()?;
    reader.LoadAsync(size).ok()?.get().ok()?;
    let mut buf = vec![0u8; size as usize];
    reader.ReadBytes(&mut buf).ok()?;
    Some(buf)
}

fn http_get(url: &str) -> Option<Vec<u8>> {
    let client = HttpClient::new().ok()?;
    let uri = Uri::CreateUri(&HSTRING::from(url)).ok()?;
    let buf = client.GetBufferAsync(&uri).ok()?.get().ok()?;
    let len = buf.Length().ok()? as usize;
    if len == 0 || len > 8_000_000 {
        return None;
    }
    let r = DataReader::FromBuffer(&buf).ok()?;
    let mut v = vec![0u8; len];
    r.ReadBytes(&mut v).ok()?;
    Some(v)
}

fn urlenc(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => o.push(b as char),
            b' ' => o.push('+'),
            _ => o.push_str(&format!("%{:02X}", b)),
        }
    }
    o
}

fn fetch_online(title: &str, artist: &str) -> Option<Vec<u8>> {
    let term = format!("{} {}", artist, title);
    let url = format!("https://itunes.apple.com/search?media=music&entity=song&limit=1&term={}", urlenc(&term));
    let json = String::from_utf8(http_get(&url)?).ok()?;
    let k = json.find("\"artworkUrl100\"")? + "\"artworkUrl100\"".len();
    let rest = &json[k..];
    let a = rest.find('"')? + 1;
    let rest = &rest[a..];
    let b = rest.find('"')?;
    let art = rest[..b].replace("100x100bb", "300x300bb");
    http_get(&art)
}

fn decode_art(bytes: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(bytes).ok()?;
    let img = img.resize_to_fill(ART, ART, image::imageops::FilterType::Triangle);
    Some(img.to_rgba8().into_raw())
}

fn post_refresh(hwnd: isize) {
    unsafe {
        let _ = PostMessageW(HWND(hwnd as *mut c_void), WM_APP, WPARAM(0), LPARAM(0));
    }
}

pub(crate) fn media_thread(hwnd: isize, shared: Arc<Mutex<Media>>, art_shared: Arc<Mutex<Option<ArtData>>>, rx: Receiver<Cmd>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let mgr = match SessionManager::RequestAsync().and_then(|a| a.get()) {
        Ok(m) => m,
        Err(_) => return,
    };
    let mut art_key = String::new();
    let mut tries = 0u32;
    let mut art_done = false;
    let mut online_started = false;
    let mut online_state = Arc::new(AtomicU8::new(0)); // 0 = berjalan/belum, 1 = berhasil, 2 = gagal
    let mut last_dur: u32 = 0;

    loop {
        if let Ok(cmd) = rx.recv_timeout(Duration::from_millis(1000)) {
            if let Ok(s) = mgr.GetCurrentSession() {
                match cmd {
                    Cmd::Prev => {
                        let _ = s.TrySkipPreviousAsync();
                    }
                    Cmd::Toggle => {
                        let _ = s.TryTogglePlayPauseAsync();
                    }
                    Cmd::Next => {
                        let _ = s.TrySkipNextAsync();
                    }
                    Cmd::Seek(f) => {
                        let ticks = (f as f64 * last_dur as f64 * 10_000_000.0) as i64;
                        let _ = s.TryChangePlaybackPositionAsync(ticks);
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(250));
        }

        let mut new = Media::default();
        let mut props: Option<MediaProps> = None;
        let mut is_spotify = false;
        if let Ok(s) = mgr.GetCurrentSession() {
            is_spotify = s
                .SourceAppUserModelId()
                .map(|h| h.to_string().to_lowercase().contains("spotify"))
                .unwrap_or(false);
            if let Ok(p) = s.TryGetMediaPropertiesAsync().and_then(|a| a.get()) {
                new.title = p.Title().map(|h| h.to_string()).unwrap_or_default();
                new.artist = p.Artist().map(|h| h.to_string()).unwrap_or_default();
                props = Some(p);
            }
            if let Ok(i) = s.GetPlaybackInfo() {
                if let Ok(st) = i.PlaybackStatus() {
                    new.playing = st == PlayStatus::Playing;
                }
            }
            if let Ok(tl) = s.GetTimelineProperties() {
                let pos = tl.Position().map(|t| t.Duration).unwrap_or(0);
                let start = tl.StartTime().map(|t| t.Duration).unwrap_or(0);
                let end = tl.EndTime().map(|t| t.Duration).unwrap_or(0);
                new.pos = ((pos - start).max(0) / 10_000_000) as u32;
                new.dur = ((end - start).max(0) / 10_000_000) as u32;
            }
        }
        last_dur = new.dur;

        let key = new.key();
        if key != art_key {
            art_key = key.clone();
            tries = 0;
            art_done = new.title.is_empty();
            online_started = false;
            online_state = Arc::new(AtomicU8::new(0));
        }
        // Spotify: thumbnail GSMTC-nya bisa memuat logo/teks Spotify, jadi pakai cover bersih dari iTunes dulu.
        // Thumbnail GSMTC hanya dipakai kalau pencarian online gagal.
        let spotify_wait = is_spotify && ONLINE_ARTWORK && online_state.load(Ordering::Relaxed) != 2;
        if spotify_wait && online_state.load(Ordering::Relaxed) == 1 {
            art_done = true;
        }
        if !art_done && tries < 6 {
            if !spotify_wait {
                tries += 1;
            }
            let bytes = if spotify_wait { None } else { props.as_ref().and_then(|p| read_thumbnail(p)) };
            if let Some(rgba) = bytes.and_then(|b| decode_art(&b)) {
                *art_shared.lock().unwrap() = Some(ArtData { key: key.clone(), rgba });
                art_done = true;
                post_refresh(hwnd);
            } else if ONLINE_ARTWORK && (spotify_wait || tries >= 3) && !online_started {
                online_started = true;
                let (t, ar, k) = (new.title.clone(), new.artist.clone(), key.clone());
                let slot = art_shared.clone();
                let state = online_state.clone();
                std::thread::spawn(move || {
                    unsafe {
                        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                    }
                    if let Some(rgba) = fetch_online(&t, &ar).and_then(|b| decode_art(&b)) {
                        *slot.lock().unwrap() = Some(ArtData { key: k, rgba });
                        state.store(1, Ordering::Relaxed);
                        post_refresh(hwnd);
                    } else {
                        state.store(2, Ordering::Relaxed);
                    }
                    trim_memory();
                });
            }
        }

        let changed = {
            let mut g = shared.lock().unwrap();
            if *g != new {
                *g = new;
                true
            } else {
                false
            }
        };
        if changed {
            post_refresh(hwnd);
        }
    }
}

// Two dominant colours are extracted ONCE per artwork (in set_art, from the already decoded 88x88 RGBA that the
// media thread produced), then cross-faded over TINT_SECS whenever the artwork changes.
type Pal = [[f32; 3]; 2];

const TINT_SECS: f32 = 0.7; // palette cross-fade duration
const TINT_PEAK: f32 = 0.45; // max opacity of the tint at its strongest point (keeps text contrast)
const TF_W: u32 = 16; // the tint is a tiny 16x24 field, bilinearly stretched over the notch (=> very smooth)
const TF_H: u32 = 24;
const HUE_BINS: usize = 24;

#[derive(Clone)]
pub(crate) struct Tint {
    from: Pal,
    to: Pal,
    from_a: f32,
    to_a: f32,
    t0: Option<Instant>,
    sig: u64, // fingerprint of the artwork the palette was extracted from
}

impl Default for Tint {
    fn default() -> Self {
        Tint { from: [[0.0; 3]; 2], to: [[0.0; 3]; 2], from_a: 0.0, to_a: 0.0, t0: None, sig: 0 }
    }
}

impl Tint {
    fn progress(&self) -> f32 {
        match self.t0 {
            Some(t) => {
                let x = (t.elapsed().as_secs_f32() / TINT_SECS).clamp(0.0, 1.0);
                x * x * (3.0 - 2.0 * x)
            }
            None => 1.0,
        }
    }

    /// Current (palette, strength), i.e. the in-flight cross-fade value.
    fn eval(&self) -> (Pal, f32) {
        let p = self.progress();
        let mut c = self.to;
        for i in 0..2 {
            for k in 0..3 {
                c[i][k] = self.from[i][k] + (self.to[i][k] - self.from[i][k]) * p;
            }
        }
        (c, self.from_a + (self.to_a - self.from_a) * p)
    }

    pub(crate) fn animating(&self) -> bool {
        self.t0.map_or(false, |t| t.elapsed().as_secs_f32() < TINT_SECS)
    }

    /// Start a transition from whatever is on screen right now to `pal` (None = fade out).
    fn retarget(&mut self, pal: Option<Pal>) {
        let (cur, cur_a) = self.eval();
        match pal {
            Some(p) => {
                self.from = if cur_a < 0.02 { p } else { cur }; // nothing visible yet -> just fade the new one in
                self.to = p;
                self.to_a = 1.0;
            }
            None => {
                self.from = cur;
                self.to = cur;
                self.to_a = 0.0;
            }
        }
        self.from_a = cur_a;
        self.t0 = Some(Instant::now());
    }
}

fn sig_of(b: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn rgb_to_hsv(c: [f32; 3]) -> (f32, f32, f32) {
    let mx = c[0].max(c[1]).max(c[2]);
    let mn = c[0].min(c[1]).min(c[2]);
    let d = mx - mn;
    let h = if d < 1e-6 {
        0.0
    } else if mx == c[0] {
        ((c[1] - c[2]) / d).rem_euclid(6.0)
    } else if mx == c[1] {
        (c[2] - c[0]) / d + 2.0
    } else {
        (c[0] - c[1]) / d + 4.0
    };
    (h * 60.0, if mx <= 0.0 { 0.0 } else { d / mx }, mx)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h.rem_euclid(360.0) / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r + m, g + m, b + m]
}

/// Pull a colour into a range that looks good as a soft tint over black (not neon, not muddy).
fn fit_tint(c: [f32; 3]) -> [f32; 3] {
    let (h, s, v) = rgb_to_hsv(c);
    let s = if s > 0.12 { (s * 1.1).clamp(0.45, 0.95) } else { s };
    let mut o = hsv_to_rgb(h, s, v.clamp(0.45, 0.72));
    let lum = 0.2126 * o[0] + 0.7152 * o[1] + 0.0722 * o[2];
    if lum > 0.55 {
        for k in o.iter_mut() {
            *k *= 0.55 / lum;
        }
    }
    o
}

/// Two dominant colours of the artwork: a hue histogram weighted by saturation/brightness (and slightly by
/// being low in the image, where the bleed happens). Near-black pixels and greys are ignored unless the whole
/// artwork is grey. Returns None if the artwork has nothing usable (e.g. fully black / transparent).
fn extract_palette(rgba: &[u8]) -> Option<Pal> {
    let n = (ART * ART) as usize;
    if rgba.len() < n * 4 {
        return None;
    }
    let mut wsum = [0f32; HUE_BINS];
    let mut csum = [[0f32; 3]; HUE_BINS];
    let (mut gw, mut gsum) = (0f32, [0f32; 3]);
    for (i, px) in rgba.chunks_exact(4).take(n).enumerate() {
        if px[3] < 128 {
            continue;
        }
        let c = [px[0] as f32 / 255.0, px[1] as f32 / 255.0, px[2] as f32 / 255.0];
        let (h, s, v) = rgb_to_hsv(c);
        if v < 0.10 {
            continue;
        }
        let y = (i / ART as usize) as f32 / ART as f32;
        let w = (0.4 + 0.6 * v) * (1.0 + 0.5 * y);
        gw += w;
        for k in 0..3 {
            gsum[k] += c[k] * w;
        }
        if s < 0.15 {
            continue;
        }
        let wc = w * (0.15 + s);
        let b = ((h / 360.0 * HUE_BINS as f32) as usize).min(HUE_BINS - 1);
        wsum[b] += wc;
        for k in 0..3 {
            csum[b][k] += c[k] * wc;
        }
    }
    if gw <= 0.0 {
        return None;
    }
    let tot: f32 = wsum.iter().sum();
    if tot < 0.05 * gw {
        // (nearly) greyscale artwork
        let avg = fit_tint([gsum[0] / gw, gsum[1] / gw, gsum[2] / gw]);
        return Some([avg, avg]);
    }
    let score = |b: usize| wsum[b] + 0.5 * (wsum[(b + 1) % HUE_BINS] + wsum[(b + HUE_BINS - 1) % HUE_BINS]);
    let colour = |b: usize| {
        let (mut w, mut c) = (0f32, [0f32; 3]);
        for d in [HUE_BINS - 1, 0, 1] {
            let j = (b + d) % HUE_BINS;
            w += wsum[j];
            for k in 0..3 {
                c[k] += csum[j][k];
            }
        }
        if w <= 0.0 {
            [0.0; 3]
        } else {
            [c[0] / w, c[1] / w, c[2] / w]
        }
    };
    let b1 = (0..HUE_BINS).max_by(|&a, &b| score(a).total_cmp(&score(b)))?;
    let s1 = score(b1);
    let dist = |a: usize, b: usize| {
        let d = (a as i32 - b as i32).unsigned_abs() as usize;
        d.min(HUE_BINS - d)
    };
    // second colour: strongest hue at least 60 degrees away, if it carries a meaningful share
    let b2 = (0..HUE_BINS).filter(|&b| dist(b, b1) >= 4).max_by(|&a, &b| score(a).total_cmp(&score(b)));
    let c1 = colour(b1);
    let c2 = match b2 {
        Some(b) if score(b) >= 0.25 * s1 => colour(b),
        _ => c1,
    };
    Some([fit_tint(c1), fit_tint(c2)])
}

impl App {
    pub(crate) fn set_art(&mut self, rgba: &[u8]) {
        self.art = pixmap_from_rgba(rgba, ART);
        if self.art.is_none() {
            return; // music_glow fades the tint out
        }
        // Extract colours only when the artwork really changed.
        let sig = sig_of(rgba);
        if sig == self.tint.sig {
            return;
        }
        self.tint.sig = sig;
        self.tint.retarget(extract_palette(rgba));
    }

    /// Artwork colour bleed for the Music slide. Drawn under the slide content (like `glow` for the other slides),
    /// clipped to the notch shape: strongest at the bottom, fading to fully transparent towards the top, with
    /// the dominant colour sitting under the artwork and the second one drifting to the right.
    pub(crate) fn music_glow(&mut self, g: &mut Gfx, shape: &Path, w: f32, h: f32, al: f32) {
        if self.art.is_none() && self.tint.to_a > 0.0 {
            // Artwork went away (new track, nothing yet / failed): fade out instead of snapping.
            self.tint.retarget(None);
            self.tint.sig = 0;
        }
        let (pal, amt) = self.tint.eval();
        let opacity = amt * al;
        if opacity <= 0.01 || w < 1.0 || h < 1.0 {
            return;
        }
        let l = CX - w / 2.0;
        let ax = ((LX + 44.0 - l) / w).clamp(0.0, 1.0); // artwork centre, as a fraction of the notch width
        let Some(mut pm) = Pixmap::new(TF_W, TF_H) else { return; };
        let data = pm.data_mut();
        for j in 0..TF_H {
            let v = (j as f32 + 0.5) / TF_H as f32;
            let f = smoothstep(0.2, 0.85, v); // 0 at the top ... full from ~85% down
            for i in 0..TF_W {
                let u = (i as f32 + 0.5) / TF_W as f32;
                let d = (u - ax) / 0.38;
                let gx = 0.55 + 0.45 * (-d * d).exp(); // a bit stronger under the artwork than behind the controls
                let m = smoothstep(ax - 0.1, ax + 0.7, u); // 0 = dominant colour, 1 = secondary
                let a = TINT_PEAK * f * gx;
                let o = ((j * TF_W + i) * 4) as usize;
                for k in 0..3 {
                    let c = pal[0][k] + (pal[1][k] - pal[0][k]) * m;
                    data[o + k] = (c * a * 255.0 + 0.5) as u8; // premultiplied
                }
                data[o + 3] = (a * 255.0 + 0.5) as u8;
            }
        }
        let mut p = Paint::default();
        p.shader = Pattern::new(
            pm.as_ref(),
            SpreadMode::Pad,
            FilterQuality::Bilinear,
            opacity.clamp(0.0, 1.0),
            Transform::from_row(w / TF_W as f32, 0.0, 0.0, h / TF_H as f32, l, 0.0),
        );
        p.anti_alias = true;
        g.fill(shape, &p);
    }

    fn note_icon(g: &mut Gfx, cx: f32, cy: f32, px: f32, al: f32) {
        let Some(p) = note_icon_path() else { return; };
        let k = px / 256.0;
        let t = Transform::from_scale(k, k).post_translate(cx - px / 2.0, cy - px / 2.0);
        if let Some(path) = p.clone().transform(t) {
            g.fill(&path, &paint(150, 150, 160, al));
        }
    }

    fn art_at(&self, g: &mut Gfx, x: f32, y: f32, size: f32, r: f32, al: f32) {
        let path = rrect(x, y, size, size, r);
        if let Some(art) = &self.art {
            let s = size / ART as f32;
            let mut p = Paint::default();
            p.shader = Pattern::new(
                art.as_ref(),
                SpreadMode::Pad,
                FilterQuality::Bilinear,
                al,
                Transform::from_row(s, 0.0, 0.0, s, x, y),
            );
            p.anti_alias = true;
            g.fill(&path, &p);
        } else {
            g.glass_panel(&path, al);
            Self::note_icon(g, x + size / 2.0, y + size / 2.0, size * 0.5, al);
        }
    }

    pub(crate) fn slide_music(&self, g: &mut Gfx, ox: f32, al: f32) {
        g.txt("Now Playing", LX + ox, 14.0, 12.0, [140, 140, 145], al, 200.0);
        self.clock_side(g, RX + ox, al);
        self.art_at(g, LX + ox, 44.0, 88.0, 12.0, al);
        let empty = self.media.title.is_empty();
        let title = if empty { "No media" } else { &self.media.title };
        g.txtb(title, MX + ox, 40.0, 17.0, [255, 255, 255], al, MW);
        if empty {
            g.txt("Start music in any app", MX + ox, 64.0, 13.0, [140, 140, 145], al, MW);
        } else {
            g.txt(&self.media.artist, MX + ox, 64.0, 13.0, [170, 170, 175], al, MW);
        }
        if self.media.dur > 0 {
            let fr = (self.media.pos as f32 / self.media.dur as f32).clamp(0.0, 1.0);
            g.fill(&rrect(MX + ox, 96.0, MW, 4.0, 2.0), &paint(255, 255, 255, 0.18 * al));
            g.fill(&rrect(MX + ox, 96.0, (MW * fr).max(4.0), 4.0, 2.0), &acc(al));
            g.txt(&fmt_time(self.media.pos), MX + ox, 104.0, 11.0, [140, 140, 145], al, 60.0);
            let ds = fmt_time(self.media.dur);
            let dw = g.width(&ds, 11.0);
            g.txt(&ds, MX + ox + MW - dw, 104.0, 11.0, [140, 140, 145], al, 60.0);
        }
        let cx = MCX + ox;
        let cy = MCY;
        // Tombol redup selama tidak ada media yang bisa dikontrol.
        let cal = if empty { al * 0.4 } else { al };
        let white = paint(255, 255, 255, cal);
        let px = cx - 56.0;
        g.poly(&[(px + 6.0, cy - 7.0), (px + 6.0, cy + 7.0), (px - 6.0, cy)], &white);
        g.rect(px - 9.0, cy - 7.0, 2.5, 14.0, &white);
        let nx = cx + 56.0;
        g.poly(&[(nx - 6.0, cy - 7.0), (nx - 6.0, cy + 7.0), (nx + 6.0, cy)], &white);
        g.rect(nx + 6.5, cy - 7.0, 2.5, 14.0, &white);
        g.circle(cx, cy, 17.0, &white);
        let blk = paint(0, 0, 0, cal);
        if self.media.playing {
            g.rect(cx - 6.0, cy - 7.0, 4.0, 14.0, &blk);
            g.rect(cx + 2.0, cy - 7.0, 4.0, 14.0, &blk);
        } else {
            g.poly(&[(cx - 4.0, cy - 8.0), (cx - 4.0, cy + 8.0), (cx + 8.0, cy)], &blk);
        }
    }
}