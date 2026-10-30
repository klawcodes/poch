#![windows_subsystem = "windows"]
//! Poch for Windows 10/11 (flush with the top edge of the screen).
//! - Layered per-pixel alpha window (UpdateLayeredWindow), software rendering (tiny-skia)
//! - Slides: Music (with clock), Water, To-do, Ledge (file shelf, max 5 items), Scrap (mini notepad). Endless carousel: arrow buttons, swipe/scroll, drag, dots
//! - Scrap: scrollable note (max 3,333 chars), caret + selection, clipboard, live character counter
//! - Busy mode: Adobe apps / maximized / fullscreen window -> notch shrinks to a thin line, hover to open
//! - Media: Windows GSMTC (Spotify, browsers, ...). Artwork from GSMTC, fallback to iTunes Search API
//! - Hotkey Ctrl+Alt+H (or "h" when focused): toggle clock <-> eye
//! - Idle: 10 min clock (time), then 15 min "blinking eye" (shapes baked from eye.svg) with a running
//!   red-shades glow that fades to transparent upwards, then loop (only counts while idle)
//! - Startup: the eye + glow show for 1 s right after launch, then cross-fade into the clock
//! - Data: %APPDATA%\poch\todo.txt, water.txt, shelf.txt and notepad.txt (todo/water migrated automatically from the old %APPDATA%\notch folder)
//! - Font: Instrument Sans (embedded in the exe, see assets/)
//! - Memory: small embedded fonts, bounded glyph cache, working set trimmed when idle
//! - Ledge: drag-out via OLE (copy default, Shift+drag = move)

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tiny_skia::{
    Color, FillRule, FilterQuality, GradientStop, LineCap, LinearGradient, Mask, Paint, Path, PathBuilder, Pattern,
    Pixmap, Point, Rect, SpreadMode, Stroke, Transform,
};
use windows::core::{w, Result, HSTRING, PCWSTR, PWSTR};
use windows::Foundation::Uri;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSessionManager as SessionManager,
    GlobalSystemMediaTransportControlsSessionMediaProperties as MediaProps,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlayStatus,
};
use windows::Storage::Streams::DataReader;
use windows::Web::Http::HttpClient;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::OleInitialize;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, QueryFullProcessImageNameW, SetProcessWorkingSetSize, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, ReleaseCapture, SetCapture, SetFocus, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
};
use windows::Win32::UI::Shell::{DragFinish, DragQueryFileW, ShellExecuteW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::*;

mod anim;
mod busy;
mod clock;
mod config;
mod data;
mod dragdrop;
mod eye;
mod gfx;
mod glass;
mod icon;
mod input;
mod menu;
mod music;
mod nav;
mod note;
mod render;
mod shelf;
mod todo;
mod water;
mod window;

use anim::{schedule, target, tick};
use busy::{is_busy};
use clock::{CLOCK_FADE_IN, DAYS_S, MONTHS, now_tm, today};
use config::{ACCENT, CL, CW, CX, DROP_AMP, DROP_ANIM_SECS, EXP_H, EXP_W, H, IDLE, LX, NAVY, N_SLIDES, PLAYING, RX, THIN, W};
use data::{data_path, migrate_old_data, trim_memory, write_data};
use eye::{EYE_REST_MS, EyeAnim, HOTKEY_ID, HOTKEY_PLAIN_H, IDLE_CYCLE_SECS, IdlePhase, SPLASH_FADE, SPLASH_SECS, build_eye_paths, splash_tick, toggle_idle_phase};
use gfx::{Box4, Gfx, Text, acc, dummy_path, ellipsize, in_rect, notch_path, notch_path_w, paint, pixmap_from_rgba, rrect, ty, union_box, wrap_lines};
use icon::{load_icon};
use input::{click, context_menu};
use music::{ArtData, Cmd, MARQUEE_GAP, MARQUEE_PAUSE, MARQUEE_SPEED, MCX, MCY, MW, MX, Media, media_thread};
use nav::{NAV_L, NAV_R, cyc, dot_x};
use note::{NOTE_H, NOTE_Y, NoteView, load_note, save_note};
use render::present;
use shelf::{SH_GAP, SH_H, SH_W, SH_Y, ShelfItem, load_shelf, open_path, save_shelf};
use todo::{ROW_H, ROW_P, load_todos, rows_geom, save_todos};
use water::{BX, Water, check_reminder};
use window::{wndproc};

struct App {
    hwnd: isize,
    wx: i32,
    mem_dc: isize,
    bits: usize,
    gfx: Option<Gfx>,
    w: f32,
    h: f32,
    vw: f32,
    vh: f32,
    slide: i32,
    sp: f32,
    sv: f32,
    drag: Option<(f32, f32)>,
    drag_dx: f32,
    shelf_drag: Option<(usize, f32, f32)>,
    last_wheel: Instant,
    t: f32,
    last: Instant,
    interval: u32,
    tick_n: u32,
    last_sec: u16,
    last_min: u16,
    hover: bool,
    busy: bool,
    menu_open: bool,
    peek_until: Option<Instant>,
    reminding: bool,
    splash: Option<Instant>,
    idle_base: f32,
    idle_mark: Option<Instant>,
    was_idle: bool,
    eye: EyeAnim,
    eye_paths: Vec<Path>,
    cur_phase: IdlePhase,
    shared: Arc<Mutex<Media>>,
    art_shared: Arc<Mutex<Option<ArtData>>>,
    media: Media,
    art: Option<Pixmap>,
    tint: music::Tint,
    icon: Option<Pixmap>,
    art_key: String,
    title_t0: f32,
    todos: Vec<(bool, String)>,
    todo_scroll: usize,
    edit_mode: bool,
    editing: bool,
    edit_idx: Option<usize>,
    input: String,
    caret_on: bool,
    edit_start: Instant,
    prev_fg: isize,
    shelf: Vec<ShelfItem>,
    shelf_flash: Option<Instant>,
    drop_pulse: Option<Instant>,
    note: String,
    note_mode: bool,
    nv: NoteView,
    water: Water,
    tx: Sender<Cmd>,
}

static APP: Mutex<Option<App>> = Mutex::new(None);

fn with_app<F: FnOnce(&mut App)>(f: F) {
    use std::sync::TryLockError;
    // Kalau pernah ada panic saat lock dipegang, mutex jadi "poisoned": tetap pakai datanya, jangan mati total.
    let mut g = match APP.try_lock() {
        Ok(g) => g,
        Err(TryLockError::Poisoned(p)) => p.into_inner(),
        Err(TryLockError::WouldBlock) => return,
    };
    if let Some(a) = g.as_mut() {
        f(a);
    }
}

impl App {
    fn expanded(&self) -> bool {
        self.hover
            || self.editing
            || self.drag.is_some()
            || self.peek_until.map_or(false, |t| Instant::now() < t)
    }

    fn idle(&self) -> bool {
        !self.expanded() && !self.busy && !self.media.playing
    }

    fn ready(&self) -> bool {
        self.h >= EXP_H - 30.0
    }
}

fn main() -> Result<()> {
    // Karena windows_subsystem = "windows", panic tidak terlihat. Catat ke file supaya penyebab crash bisa dilacak.
    std::panic::set_hook(Box::new(|info| {
        let line = format!("{:?} | {}\n", std::time::SystemTime::now(), info);
        let p = data::data_path("crash.log");
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
            let _ = f.write_all(line.as_bytes());
        }
    }));
    unsafe {
        let _ = SetProcessDPIAware();
        let hinst = GetModuleHandleW(None)?;
        let class = w!("PochWindow");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        RegisterClassW(&wc);

        let sw = GetSystemMetrics(SM_CXSCREEN);
        let wx = (sw - W) / 2;
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_ACCEPTFILES,
            class,
            w!("Poch"),
            WS_POPUP,
            wx,
            0,
            W,
            H,
            None,
            None,
            hinst,
            None,
        )?;

        let mods = if HOTKEY_PLAIN_H {
            MOD_NOREPEAT
        } else {
            HOT_KEY_MODIFIERS(MOD_ALT.0 | MOD_CONTROL.0 | MOD_NOREPEAT.0)
        };
        let _ = RegisterHotKey(hwnd, HOTKEY_ID, mods, 'H' as u32);

        let screen = GetDC(None);
        let mem = CreateCompatibleDC(screen);
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
        let bmp = CreateDIBSection(mem, &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
        SelectObject(mem, bmp);
        ReleaseDC(None, screen);

        // OLE dibutuhkan oleh DoDragDrop (drag-out Ledge). Aman dipanggil sekali di UI thread.
        let _ = OleInitialize(None);

        let shared = Arc::new(Mutex::new(Media::default()));
        let art_shared: Arc<Mutex<Option<ArtData>>> = Arc::new(Mutex::new(None));
        let (tx, rx) = channel();
        {
            let sh = shared.clone();
            let ar = art_shared.clone();
            let h = hwnd.0 as isize;
            std::thread::spawn(move || media_thread(h, sh, ar, rx));
        }

        migrate_old_data();
        let st = now_tm();
        let mut app = App {
            hwnd: hwnd.0 as isize,
            wx,
            mem_dc: mem.0 as isize,
            bits: bits as usize,
            gfx: Some(Gfx {
                pm: Pixmap::new(W as u32, H as u32).unwrap(),
                text: Text::load(),
                mask: Mask::new(W as u32, H as u32),
                prev_box: (0, 0, W as usize, H as usize),
                dirty: (0, 0, W as usize, H as usize),
            }),
            w: IDLE.0,
            h: IDLE.1,
            vw: 0.0,
            vh: 0.0,
            slide: 0,
            sp: 0.0,
            sv: 0.0,
            drag: None,
            drag_dx: 0.0,
            shelf_drag: None,
            last_wheel: Instant::now(),
            t: 0.0,
            last: Instant::now(),
            interval: 0,
            tick_n: 0,
            last_sec: st.wSecond,
            last_min: st.wMinute,
            hover: false,
            busy: is_busy(hwnd),
            menu_open: false,
            peek_until: None,
            reminding: false,
            splash: Some(Instant::now()),
            idle_base: 0.0,
            idle_mark: None,
            was_idle: false,
            eye: {
                let mut e = EyeAnim::new();
                e.hold = 0.18; // shorter closed-eyes hold so the pop fits inside the 1 s splash
                e
            },
            eye_paths: build_eye_paths(),
            cur_phase: IdlePhase::Clock,
            shared,
            art_shared,
            media: Media::default(),
            art: None,
            tint: music::Tint::default(),
            icon: load_icon(),
            art_key: String::new(),
            title_t0: 0.0,
            todos: load_todos(),
            todo_scroll: 0,
            edit_mode: false,
            editing: false,
            edit_idx: None,
            input: String::new(),
            caret_on: true,
            edit_start: Instant::now(),
            prev_fg: 0,
            shelf: load_shelf(),
            shelf_flash: None,
            drop_pulse: None,
            note: load_note(),
            note_mode: false,
            nv: NoteView::default(),
            water: Water::load(),
            tx,
        };
        let (tw, th) = target(&app);
        app.w = tw;
        app.h = th;
        present(&mut app);
        *APP.lock().unwrap() = Some(app);

        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetTimer(hwnd, 2, 100, None);
        trim_memory();

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}