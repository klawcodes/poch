//! Layout & ukuran bersama (dipakai semua fitur).

pub(crate) const W: i32 = 540;
pub(crate) const H: i32 = 192; // +12 px ruang transparan untuk gelombang saat drop
pub(crate) const EXP_W: f32 = 500.0;
pub(crate) const EXP_H: f32 = 172.0;
pub(crate) const CX: f32 = 270.0;
pub(crate) const CL: f32 = 20.0;
pub(crate) const LX: f32 = CL + 48.0;
pub(crate) const RX: f32 = CL + EXP_W - 48.0;
pub(crate) const CW: f32 = RX - LX;
pub(crate) const NAVY: f32 = 92.0;
pub(crate) const N_SLIDES: usize = 5; // 0 Music (+ jam), 1 Water, 2 To-do, 3 Ledge, 4 Scrap

pub(crate) const THIN: (f32, f32) = (96.0, 4.0);
pub(crate) const IDLE: (f32, f32) = (150.0, 32.0);
pub(crate) const PLAYING: (f32, f32) = (236.0, 32.0);

pub(crate) const ACCENT: [u8; 3] = [255, 69, 58];

// Animasi gelombang saat item di-drop ke Ledge: seluruh tepi bawah notch melorot,
// lalu riak merambat dari tengah ke kiri-kanan sambil meredam.
pub(crate) const DROP_ANIM_SECS: f32 = 1.6;
pub(crate) const DROP_AMP: f32 = 22.0;

// Crystal glass look: light dark tint, brighter top sheen, nested panel tint.
pub(crate) const GLASS_ALPHA: f32 = 0.52;
pub(crate) const GLASS_SHEEN: f32 = 0.18;
pub(crate) const GLASS_PANEL: f32 = 0.30;
// DWM blur-behind stacks a second, thick blur on top of the one in glass.rs, so it is off.
pub(crate) const GLASS_BLUR: bool = false;