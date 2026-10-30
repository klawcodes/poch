//! Fitur Ledge: rak file (maks 5 item), drag & drop, slide Ledge.

use super::*;

use windows::core::Interface;
use windows::Win32::System::Com::IBindCtx;
use windows::Win32::UI::Shell::{IShellItem, IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_ICONONLY};
use tiny_skia::PixmapPaint;

const SHELF_NAME: &str = "Ledge";

const SHELF_MAX: usize = 5; // folder / zip dihitung 1 item
pub(crate) const SH_GAP: f32 = 8.0;
pub(crate) const SH_W: f32 = (CW - SH_GAP * 4.0) / SHELF_MAX as f32;
pub(crate) const SH_Y: f32 = 38.0;
pub(crate) const SH_H: f32 = 100.0;

const ICON_PX: i32 = 48;

pub(crate) struct ShelfItem {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) icon: Option<Pixmap>,
}

impl ShelfItem {
    pub(crate) fn new(path: &str) -> Self {
        let name = std::path::Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string());
        ShelfItem { path: path.to_string(), name, icon: load_file_icon(path) }
    }
}

/// Ikon berwarna dari Windows Shell (sama dengan yang terlihat di Explorer).
/// Dimuat sekali saat item dibuat, bukan setiap frame.
fn load_file_icon(path: &str) -> Option<Pixmap> {
    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(path), None::<&IBindCtx>).ok()?;
        let factory: IShellItemImageFactory = item.cast().ok()?;
        let hbmp = factory.GetImage(SIZE { cx: ICON_PX, cy: ICON_PX }, SIIGBF_ICONONLY).ok()?;

        let mut bm = BITMAP::default();
        GetObjectW(
            HGDIOBJ(hbmp.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut c_void),
        );
        let (w, h) = (bm.bmWidth, bm.bmHeight);
        if w <= 0 || h <= 0 {
            let _ = DeleteObject(hbmp);
            return None;
        }
        let mut bmi = BITMAPINFO {
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
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let dc = GetDC(None);
        let n = GetDIBits(dc, hbmp, 0, h as u32, Some(buf.as_mut_ptr() as *mut c_void), &mut bmi, DIB_RGB_COLORS);
        ReleaseDC(None, dc);
        let _ = DeleteObject(hbmp);
        if n == 0 {
            return None;
        }

        // Jika tidak ada alpha sama sekali, anggap opaque.
        let has_alpha = buf.chunks_exact(4).any(|p| p[3] != 0);
        let mut pm = Pixmap::new(w as u32, h as u32)?;
        for (d, s) in pm.data_mut().chunks_exact_mut(4).zip(buf.chunks_exact(4)) {
            d[0] = s[2]; // BGRA -> RGBA
            d[1] = s[1];
            d[2] = s[0];
            d[3] = if has_alpha { s[3] } else { 255 };
        }
        Some(pm)
    }
}

pub(crate) fn load_shelf() -> Vec<ShelfItem> {
    std::fs::read_to_string(data_path("shelf.txt"))
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).take(SHELF_MAX).map(ShelfItem::new).collect())
        .unwrap_or_default()
}

pub(crate) fn save_shelf(v: &[ShelfItem]) {
    write_data("shelf.txt", v.iter().map(|i| format!("{}\n", i.path)).collect());
}

pub(crate) fn open_path(path: &str) {
    unsafe {
        let _ = ShellExecuteW(
            HWND::default(),
            w!("open"),
            &HSTRING::from(path),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

impl App {
    /// Tambah path hasil drop ke shelf (duplikat diabaikan). Set flash "full" bila limit tercapai.
    pub(crate) fn shelf_add(&mut self, paths: Vec<String>) {
        let mut changed = false;
        for p in paths {
            if self.shelf.iter().any(|s| s.path.eq_ignore_ascii_case(&p)) {
                continue;
            }
            if self.shelf.len() >= SHELF_MAX {
                self.shelf_flash = Some(Instant::now());
                break;
            }
            self.shelf.push(ShelfItem::new(&p));
            changed = true;
        }
        if !changed {
            return;
        }
        save_shelf(&self.shelf);
        // Picu animasi droplet: tepi bawah notch melorot lalu memantul kembali.
        self.drop_pulse = Some(Instant::now());
        self.last = Instant::now();
        unsafe {
            schedule(self);
            present(self);
        }
    }

    /// Detik sejak item di-drop selama animasi gelombang berjalan; None bila tidak ada animasi.
    pub(crate) fn drop_t(&self) -> Option<f32> {
        let e = self.drop_pulse?.elapsed().as_secs_f32();
        if e < DROP_ANIM_SECS { Some(e) } else { None }
    }

    /// (Tidak dipakai lagi oleh render; dipertahankan agar kode lain tidak putus.)
    /// Besar lendutan tepi bawah notch (px) untuk pulse droplet saat ini.
    /// Positif = melorot ke bawah, negatif = memantul ke atas; 0 = bentuk normal.
    pub(crate) fn drop_dip(&self) -> f32 {
        let Some(t) = self.drop_pulse else { return 0.0; };
        let e = t.elapsed().as_secs_f32();
        if e >= DROP_ANIM_SECS {
            return 0.0;
        }
        // Pegas teredam: jatuh cepat, memantul sedikit ke atas, lalu tenang.
        let spring = (-4.2 * e).exp() * (std::f32::consts::TAU * 1.9 * e).sin();
        let ph = e / DROP_ANIM_SECS; // 0..1
        let fade = 1.0 - ((ph - 0.75) / 0.25).clamp(0.0, 1.0); // ujung animasi mulus ke 0
        spring * fade * DROP_AMP
    }

    pub(crate) fn slide_shelf(&self, g: &mut Gfx, ox: f32, al: f32) {
        let x = LX + ox;
        g.txtb(SHELF_NAME, x, 12.0, 15.0, [255, 255, 255], al, 100.0);

        let full = self.shelf_flash.map_or(false, |t| t.elapsed() < Duration::from_millis(1800));
        let label = if full {
            format!("{} is full", SHELF_NAME)
        } else {
            format!("{}/{}", self.shelf.len(), SHELF_MAX)
        };
        let lw = g.width_ex(&label, 13.0, true);
        g.txtb(&label, RX + ox - lw, 14.0, 13.0, ACCENT, al, 160.0);

        // Drop zone: satu outline putih transparan, tanpa kartu per-item.
        g.stroke(&rrect(x, SH_Y, CW, SH_H, 14.0), &paint(255, 255, 255, 0.16 * al), 1.2);

        if self.shelf.is_empty() {
            g.txt_c(
                "Drop files or folders here",
                CX + ox,
                SH_Y + SH_H / 2.0 - 9.0,
                13.0,
                [140, 140, 145],
                al,
                300.0,
                false,
            );
            return;
        }

        for i in 0..self.shelf.len() {
            let tx = x + i as f32 * (SH_W + SH_GAP);
            let it = &self.shelf[i];
            let cx = tx + SH_W / 2.0;

            if let Some(ic) = &it.icon {
                let ix = (cx - ic.width() as f32 / 2.0).round() as i32;
                let iy = (SH_Y + 10.0) as i32;
                g.pm.draw_pixmap(
                    ix,
                    iy,
                    ic.as_ref(),
                    &PixmapPaint { opacity: al, ..Default::default() },
                    Transform::identity(),
                    None,
                );
            }

            // Nama file di bawah ikon (maks 2 baris).
            let lines = wrap_lines(g, &it.name, 11.0, SH_W - 12.0);
            let y0 = SH_Y + 64.0;
            for (li, l) in lines.iter().take(2).enumerate() {
                let s = if li == 1 && lines.len() > 2 { ellipsize(g, l, 11.0, SH_W - 12.0) } else { l.clone() };
                g.txt_c(&s, cx, y0 + li as f32 * 14.0, 11.0, [235, 235, 238], al, SH_W - 8.0, false);
            }

            // Tombol hapus kecil (posisinya sama dengan hit-area di input.rs).
            let (bx, by) = (tx + SH_W - 10.0, SH_Y + 10.0);
            let wh = paint(255, 255, 255, 0.5 * al);
            g.line(bx - 2.4, by - 2.4, bx + 2.4, by + 2.4, 1.3, &wh);
            g.line(bx - 2.4, by + 2.4, bx + 2.4, by - 2.4, 1.3, &wh);
        }
    }
}