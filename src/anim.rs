//! Loop animasi: ukuran target notch, timer, polling hover/busy/media.

use super::*;

pub(crate) fn target(a: &App) -> (f32, f32) {
    if a.expanded() {
        (EXP_W, EXP_H)
    } else if a.busy {
        THIN
    } else if a.media.playing {
        PLAYING
    } else {
        IDLE
    }
}

fn settled(a: &App) -> bool {
    let (tw, th) = target(a);
    (a.w - tw).abs() < 0.3
        && (a.h - th).abs() < 0.3
        && a.vw.abs() < 2.0
        && a.vh.abs() < 2.0
        && (a.sp - a.slide as f32).abs() < 0.003
        && a.sv.abs() < 0.05
}

/// Timer: 16 ms saat bergerak / animasi droplet / mid-blink, 40 ms saat mata istirahat (glow mengalir),
/// 33 ms untuk equalizer, dan mati kalau tidak ada yang bergerak.
pub(crate) unsafe fn schedule(a: &mut App) {
    let pulsing = a.drop_pulse.map_or(false, |t| t.elapsed().as_secs_f32() < DROP_ANIM_SECS);
    let want = if pulsing {
        16
    } else if !settled(a) {
        16
    } else if a.expanded() && a.cur() == 0 && a.tint.animating() {
        16 // artwork colour cross-fade on the Music slide
    } else if a.media.playing && !a.expanded() && !a.busy {
        33
    } else if a.splash_t().is_some() && !a.media.playing {
        if a.eye.active() {
            16
        } else {
            EYE_REST_MS
        }
    } else if a.idle() && a.idle_phase() == IdlePhase::Eye {
        // 60 fps saat berkedip / glance, santai saat istirahat.
        if a.eye.active() {
            16
        } else {
            EYE_REST_MS
        }
    } else if a.idle() && a.idle_phase() == IdlePhase::Clock && a.idle_secs() < CLOCK_FADE_IN + 0.1 {
        33 // clock fade-in right after the eye phase
    } else {
        0
    };
    if want != a.interval {
        let hwnd = HWND(a.hwnd as *mut c_void);
        if want == 0 {
            let _ = KillTimer(hwnd, 1);
            trim_memory();
        } else {
            SetTimer(hwnd, 1, want, None);
        }
        a.interval = want;
    }
}

unsafe fn poll(a: &mut App) {
    a.tick_n = a.tick_n.wrapping_add(1);
    let mut changed = false;
    let mut redraw = false;
    if splash_tick(a) {
        redraw = true;
    }

    // Animasi droplet selesai -> lepas state
    if let Some(t) = a.drop_pulse {
        if t.elapsed().as_secs_f32() >= DROP_ANIM_SECS {
            a.drop_pulse = None;
        }
        redraw = true;
    }

    // Idle clock: start/stop the stopwatch, wrap it once per clock+eye cycle.
    let now = Instant::now();
    let idle_now = a.idle();
    if idle_now {
        // The idle stopwatch only runs while idle, and not during the startup splash.
        if a.idle_mark.is_none() && a.splash.is_none() {
            a.idle_mark = Some(now);
        }
    } else if let Some(m) = a.idle_mark.take() {
        a.idle_base += (now - m).as_secs_f32();
    }
    let secs = a.idle_secs();
    if secs >= IDLE_CYCLE_SECS {
        a.idle_base = secs % IDLE_CYCLE_SECS;
        if a.idle_mark.is_some() {
            a.idle_mark = Some(now);
        }
    }
    let cur_phase = a.idle_phase();
    let resumed = idle_now && !a.was_idle;
    a.was_idle = idle_now;
    if cur_phase != a.cur_phase || resumed {
        a.cur_phase = cur_phase;
        if cur_phase == IdlePhase::Eye {
            a.eye = EyeAnim::new(); // eyes pop open again
        }
        redraw = true;
    }

    if !a.menu_open {
        let mut p = POINT::default();
        if GetCursorPos(&mut p).is_ok() {
            let (zw, zh) = if a.hover || a.expanded() {
                (EXP_W + 20.0, EXP_H + 8.0)
            } else if a.busy {
                (a.w.max(THIN.0) + 60.0, 10.0)
            } else {
                (a.w.max(IDLE.0) + 12.0, a.h.max(IDLE.1) + 6.0)
            };
            let x = (p.x - a.wx) as f32;
            let y = p.y as f32;
            let inside = (x - CX).abs() <= zw / 2.0 && y >= 0.0 && y <= zh;
            if inside != a.hover {
                a.hover = inside;
                changed = true;
            }
        }
    }
    if a.tick_n % 5 == 0 {
        let b = is_busy(HWND(a.hwnd as *mut c_void));
        if b != a.busy {
            a.busy = b;
            changed = true;
        }
    }
    // Selalu paling atas: kalau flag topmost hilang, ATAU ada window lain yang terlihat di atas Poch
    // di z-order (taskbar Windows, menu bar kaca-shell, taskbar pihak ketiga, dll), naikkan lagi
    // tanpa merebut fokus. Dicek tiap poll (100 ms), murah.
    {
        let hwnd = HWND(a.hwnd as *mut c_void);
        let my_thread = windows::Win32::System::Threading::GetCurrentThreadId();
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        let mut covered = ex & WS_EX_TOPMOST.0 == 0;
        if !covered {
            let mut p = GetWindow(hwnd, GW_HWNDPREV).unwrap_or_default();
            let mut guard = 0;
            while !p.is_invalid() && guard < 128 {
                if IsWindowVisible(p).as_bool() && GetWindowThreadProcessId(p, None) != my_thread {
                    covered = true;
                    break;
                }
                p = GetWindow(p, GW_HWNDPREV).unwrap_or_default();
                guard += 1;
            }
        }
        if covered {
            let _ = SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }
    if a.tick_n % 50 == 0 && a.water.roll() {
        redraw = true;
    }
    if a.tick_n % 300 == 0 {
        trim_memory();
    }
    if let Some(t) = a.peek_until {
        if Instant::now() >= t {
            a.peek_until = None;
            a.reminding = false;
            changed = true;
        }
    }
    if check_reminder(a) {
        changed = true;
    }

    let st = now_tm();
    let expanded = a.expanded();
    let cur = a.cur();
    // Slide 0 (Music + jam) dan slide 2 (To-do) menampilkan menit/tanggal.
    if expanded && (cur == 0 || cur == 2) && st.wMinute != a.last_min {
        redraw = true;
    }
    if !expanded && !a.busy && !a.media.playing && cur_phase == IdlePhase::Clock && st.wMinute != a.last_min {
        redraw = true;
    }
    a.last_sec = st.wSecond;
    a.last_min = st.wMinute;

    if let Some(t) = a.shelf_flash {
        if t.elapsed() >= Duration::from_millis(1800) {
            a.shelf_flash = None;
            redraw = true;
        }
    }
    if a.editing {
        let on = (a.edit_start.elapsed().as_millis() / 500) % 2 == 0;
        if on != a.caret_on {
            a.caret_on = on;
            redraw = true;
        }
    }

    if changed || redraw {
        present(a);
    }
    if changed {
        a.last = Instant::now();
    }
    schedule(a);
}

pub(crate) unsafe fn tick(id: usize) {
    with_app(|a| {
        if id == 2 {
            poll(a);
            return;
        }
        splash_tick(a);
        let now = Instant::now();
        let dt = (now - a.last).as_secs_f32().min(0.05);
        a.last = now;
        a.t += dt;

        // Only advance the eye once the notch is back to its small size, so the pop-open is actually seen.
        let eye_on = (a.splash_t().is_some() && !a.media.playing) || (a.idle() && a.idle_phase() == IdlePhase::Eye);
        if eye_on && a.h < IDLE.1 + 10.0 {
            a.eye.update(dt);
        }

        let (tw, th) = target(a);
        const K: f32 = 380.0;
        const C: f32 = 40.0;
        const SK: f32 = 320.0;
        const SC: f32 = 36.0;
        let n = ((dt / 0.008).ceil() as i32).max(1);
        let sdt = dt / n as f32;
        for _ in 0..n {
            a.vw += (K * (tw - a.w) - C * a.vw) * sdt;
            a.vh += (K * (th - a.h) - C * a.vh) * sdt;
            a.w += a.vw * sdt;
            a.h += a.vh * sdt;
            a.sv += (SK * (a.slide as f32 - a.sp) - SC * a.sv) * sdt;
            a.sp += a.sv * sdt;
        }
        a.w = a.w.max(20.0);
        a.h = a.h.max(2.0);
        if settled(a) {
            a.w = tw;
            a.h = th;
            a.vw = 0.0;
            a.vh = 0.0;
            a.sp = a.slide as f32;
            a.sv = 0.0;
            let k = a.slide.div_euclid(N_SLIDES as i32) * N_SLIDES as i32;
            a.slide -= k;
            a.sp -= k as f32;
        }
        present(a);
        schedule(a);
    });
}