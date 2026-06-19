// ---------------------------------------------------------------------------
// Windows Taskbar thumbnail toolbar controls (ITaskbarList3 thumb buttons).
//
// Adds Previous / Play-Pause / Next buttons to the small toolbar shown inside
// the taskbar thumbnail preview (hover the taskbar icon). souvlaki only wires up
// the SMTC overlay + media keys, so this is a separate native integration.
//
// It deliberately reuses the existing plumbing instead of adding new paths:
//   * a button click emits the same `media-control` event the SMTC handler does,
//     so the frontend's `handleMediaControl` dispatches it with no extra code;
//   * the Play/Pause icon is flipped from `smtc_set_playback`, which the
//     frontend already calls on every play-state change.
//
// COM/Win32 objects (ITaskbarList3, HWND, HICON) are apartment/thread bound and
// are only ever touched on the main UI thread: the subclass procedure runs on
// the thread that pumps the window's messages, and command handlers hop onto it
// via `run_on_main_thread`. The `unsafe Send/Sync` on the controller upholds
// that invariant exactly like the SMTC `MediaController`.
// ---------------------------------------------------------------------------

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager};

use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, TRUE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, GetDC, ReleaseDC, BITMAPINFO, BITMAPINFOHEADER,
    DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD,
};
use windows::Win32::UI::Shell::{
    DefSubclassProc, ITaskbarList3, SetWindowSubclass, TaskbarList, THBF_ENABLED, THBN_CLICKED,
    THB_FLAGS, THB_ICON, THB_TOOLTIP, THUMBBUTTON, THUMBBUTTONMASK,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, DestroyIcon, GetSystemMetrics, RegisterWindowMessageW, HICON, ICONINFO,
    SM_CXSMICON, WM_COMMAND, WM_SETTINGCHANGE,
};

// Command IDs reported in WM_COMMAND's LOWORD when a thumb button is clicked.
// Kept in a distinctive range to avoid colliding with any accelerator IDs.
const ID_PREV: u32 = 0xB001;
const ID_PLAYPAUSE: u32 = 0xB002;
const ID_NEXT: u32 = 0xB003;

// The `TaskbarButtonCreated` registered window message value, learned once at
// init and read by the subclass procedure (0 = not yet registered).
static WM_TASKBAR_BUTTON_CREATED: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Copy)]
enum Glyph {
    Prev,
    Play,
    Pause,
    Next,
}

// Live state, owned by the managed `ThumbbarController` and only mutated on the
// main UI thread.
struct Inner {
    hwnd: HWND,
    taskbar: ITaskbarList3,
    icon_prev: HICON,
    icon_play: HICON,
    icon_pause: HICON,
    icon_next: HICON,
    // Whether ThumbBarAddButtons has succeeded for the current taskbar button.
    added: bool,
    // Mirrors the player's play-state so the middle button shows the right glyph.
    playing: bool,
    // Cached system theme used to pick a glyph color that reads on the flyout.
    light_theme: bool,
}

pub struct ThumbbarController(Mutex<Option<Inner>>);
// SAFETY: the inner Win32/COM handles are only ever accessed on the main thread
// (see the module header). The Mutex guards against logical races only.
unsafe impl Send for ThumbbarController {}
unsafe impl Sync for ThumbbarController {}

impl ThumbbarController {
    pub fn new() -> Self {
        ThumbbarController(Mutex::new(None))
    }
}

// ---- glyph rasterization ---------------------------------------------------

fn edge(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    (px - bx) * (ay - by) - (ax - bx) * (py - by)
}

fn in_tri(
    px: f32,
    py: f32,
    ax: f32,
    ay: f32,
    bx: f32,
    by: f32,
    cx: f32,
    cy: f32,
) -> bool {
    let d1 = edge(px, py, ax, ay, bx, by);
    let d2 = edge(px, py, bx, by, cx, cy);
    let d3 = edge(px, py, cx, cy, ax, ay);
    let has_neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let has_pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(has_neg && has_pos)
}

// Is the point (x,y) in [0,1]^2 inside the given glyph's shape?
fn glyph_hit(k: Glyph, x: f32, y: f32) -> bool {
    let p = 0.20; // top/bottom padding
    match k {
        Glyph::Play => in_tri(x, y, 0.30, p, 0.30, 1.0 - p, 0.78, 0.5),
        Glyph::Pause => {
            (x >= 0.30 && x <= 0.44 && y >= p && y <= 1.0 - p)
                || (x >= 0.56 && x <= 0.70 && y >= p && y <= 1.0 - p)
        }
        Glyph::Prev => {
            (x >= 0.22 && x <= 0.32 && y >= p && y <= 1.0 - p)
                || in_tri(x, y, 0.74, p, 0.74, 1.0 - p, 0.36, 0.5)
        }
        Glyph::Next => {
            (x >= 0.68 && x <= 0.78 && y >= p && y <= 1.0 - p)
                || in_tri(x, y, 0.26, p, 0.26, 1.0 - p, 0.64, 0.5)
        }
    }
}

// Rasterize a glyph into a 32-bpp HICON with straight (non-premultiplied) alpha.
// Edges are 3x3 supersampled so they stay smooth at small sizes. Returns a null
// HICON on failure; the button simply shows no image in that case.
unsafe fn make_icon(kind: Glyph, size: i32, color_rgb: u32) -> HICON {
    let w = size.max(8);
    let h = size.max(8);
    let n = (w * h) as usize;

    let r = (color_rgb >> 16) & 0xFF;
    let g = (color_rgb >> 8) & 0xFF;
    let b = color_rgb & 0xFF;

    let ss = 3i32; // supersampling factor per axis
    let total = (ss * ss) as u32;
    let mut px = vec![0u32; n];
    for y in 0..h {
        for x in 0..w {
            let mut cov = 0u32;
            for sy in 0..ss {
                for sx in 0..ss {
                    let fx = (x as f32 + (sx as f32 + 0.5) / ss as f32) / w as f32;
                    let fy = (y as f32 + (sy as f32 + 0.5) / ss as f32) / h as f32;
                    if glyph_hit(kind, fx, fy) {
                        cov += 1;
                    }
                }
            }
            let a = cov * 255 / total;
            // Memory layout is BGRA; u32 = (A<<24)|(R<<16)|(G<<8)|B yields that.
            px[(y * w + x) as usize] = (a << 24) | (r << 16) | (g << 8) | b;
        }
    }

    let hdc = GetDC(None);
    let mut bmi = BITMAPINFO::default();
    bmi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = w;
    bmi.bmiHeader.biHeight = -h; // negative = top-down
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = 0; // BI_RGB
    let mut bits: *mut c_void = null_mut();
    let dib = CreateDIBSection(Some(hdc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0);
    let _ = ReleaseDC(None, hdc);

    let hbm_color = match dib {
        Ok(h) if !h.is_invalid() => h,
        _ => return HICON::default(),
    };
    if !bits.is_null() {
        std::ptr::copy_nonoverlapping(px.as_ptr() as *const u8, bits as *mut u8, n * 4);
    }

    // A zeroed AND mask; ignored in favor of the per-pixel alpha above.
    let stride_words = ((w as usize + 15) / 16) * 2; // bytes per row, word-aligned
    let mask_bytes = vec![0u8; stride_words * h as usize];
    let hbm_mask = CreateBitmap(w, h, 1, 1, Some(mask_bytes.as_ptr() as *const c_void));

    let ii = ICONINFO {
        fIcon: TRUE,
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: hbm_mask,
        hbmColor: hbm_color,
    };
    let hicon = CreateIconIndirect(&ii).unwrap_or_default();

    let _ = DeleteObject(HGDIOBJ(hbm_color.0));
    let _ = DeleteObject(HGDIOBJ(hbm_mask.0));
    hicon
}

// Light apps/taskbar theme → dark glyphs; dark theme → light glyphs. Reads the
// per-user setting; defaults to "dark theme" (light glyphs) when unset.
fn system_uses_light_theme() -> bool {
    let mut data: u32 = 0;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut data as *mut u32 as *mut c_void),
            Some(&mut size),
        )
    };
    status.is_ok() && data != 0
}

fn glyph_color(light_theme: bool) -> u32 {
    if light_theme {
        0x1E1E1E // near-black for light flyout
    } else {
        0xECECEC // near-white for dark flyout
    }
}

impl Inner {
    unsafe fn build_icons(&mut self, size: i32) {
        let color = glyph_color(self.light_theme);
        self.icon_prev = make_icon(Glyph::Prev, size, color);
        self.icon_play = make_icon(Glyph::Play, size, color);
        self.icon_pause = make_icon(Glyph::Pause, size, color);
        self.icon_next = make_icon(Glyph::Next, size, color);
    }

    fn buttons(&self) -> [THUMBBUTTON; 3] {
        let (mid_icon, mid_tip) = if self.playing {
            (self.icon_pause, "Pause")
        } else {
            (self.icon_play, "Play")
        };
        [
            make_button(ID_PREV, self.icon_prev, "Previous"),
            make_button(ID_PLAYPAUSE, mid_icon, mid_tip),
            make_button(ID_NEXT, self.icon_next, "Next"),
        ]
    }

    // Add the buttons for a freshly created taskbar button, or update them if
    // they already exist (AddButtons is a one-shot per taskbar button).
    unsafe fn add_or_update(&mut self) {
        let buttons = self.buttons();
        if self.taskbar.ThumbBarAddButtons(self.hwnd, &buttons).is_ok() {
            self.added = true;
        } else {
            let _ = self.taskbar.ThumbBarUpdateButtons(self.hwnd, &buttons);
        }
    }

    unsafe fn update(&self) {
        let buttons = self.buttons();
        let _ = self.taskbar.ThumbBarUpdateButtons(self.hwnd, &buttons);
    }

    unsafe fn destroy_icons(&self) {
        for icon in [self.icon_prev, self.icon_play, self.icon_pause, self.icon_next] {
            if !icon.is_invalid() {
                let _ = DestroyIcon(icon);
            }
        }
    }
}

fn make_button(id: u32, icon: HICON, tip: &str) -> THUMBBUTTON {
    let mut b = THUMBBUTTON {
        dwMask: THUMBBUTTONMASK(THB_ICON.0 | THB_TOOLTIP.0 | THB_FLAGS.0),
        iId: id,
        iBitmap: 0,
        hIcon: icon,
        szTip: [0u16; 260],
        dwFlags: THBF_ENABLED,
    };
    for (i, c) in tip.encode_utf16().take(259).enumerate() {
        b.szTip[i] = c;
    }
    b
}

// ---- public entry points ---------------------------------------------------

// Wire up the thumbnail toolbar: create ITaskbarList3, build icons, install the
// window subclass, and register the `TaskbarButtonCreated` message. Buttons are
// actually added once that message arrives (the taskbar button must exist), with
// an immediate attempt in case it was created before we subclassed.
pub fn init(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    // `hwnd().0` may be isize- or pointer-shaped; the double cast tolerates both.
    let hwnd_ptr = match window.hwnd() {
        Ok(h) => h.0 as isize as *mut c_void,
        Err(_) => return,
    };
    let hwnd = HWND(hwnd_ptr);

    let taskbar: ITaskbarList3 =
        match unsafe { CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER) } {
            Ok(t) => t,
            Err(_) => return,
        };
    if unsafe { taskbar.HrInit() }.is_err() {
        return;
    }

    let msg = unsafe { RegisterWindowMessageW(w!("TaskbarButtonCreated")) };
    WM_TASKBAR_BUTTON_CREATED.store(msg, Ordering::SeqCst);

    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
    let mut inner = Inner {
        hwnd,
        taskbar,
        icon_prev: HICON::default(),
        icon_play: HICON::default(),
        icon_pause: HICON::default(),
        icon_next: HICON::default(),
        added: false,
        playing: false,
        light_theme: system_uses_light_theme(),
    };
    unsafe { inner.build_icons(size) };
    // If the taskbar button already exists this succeeds now; otherwise it is a
    // no-op and the WM_TASKBARBUTTONCREATED handler does it.
    unsafe { inner.add_or_update() };

    if let Some(controller) = app.try_state::<ThumbbarController>() {
        *controller.0.lock().unwrap() = Some(inner);
    }

    // Leak one AppHandle clone (lives for the whole process) as the subclass
    // refdata so the procedure can emit events and reach the controller.
    let boxed = Box::new(app.clone());
    let refdata = Box::into_raw(boxed) as usize;
    unsafe {
        let _ = SetWindowSubclass(hwnd, Some(subclass_proc), 1, refdata);
    }
}

// Flip the middle button between Play and Pause. Called from `smtc_set_playback`
// and marshalled onto the main thread before touching any Win32 state.
pub fn set_playing(app: &AppHandle, playing: bool) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(controller) = app2.try_state::<ThumbbarController>() {
            if let Ok(mut guard) = controller.0.lock() {
                if let Some(inner) = guard.as_mut() {
                    if inner.playing != playing {
                        inner.playing = playing;
                        if inner.added {
                            unsafe { inner.update() };
                        }
                    }
                }
            }
        }
    });
}

// Does `lparam` point at the given wide string? Used to filter WM_SETTINGCHANGE.
unsafe fn lparam_is(lparam: LPARAM, expected: &str) -> bool {
    if lparam.0 == 0 {
        return false;
    }
    let p = lparam.0 as *const u16;
    let mut len = 0usize;
    while len < 256 && *p.add(len) != 0 {
        len += 1;
    }
    let slice = std::slice::from_raw_parts(p, len);
    String::from_utf16_lossy(slice) == expected
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _uid_subclass: usize,
    refdata: usize,
) -> LRESULT {
    let app = &*(refdata as *const AppHandle);

    let tbc_msg = WM_TASKBAR_BUTTON_CREATED.load(Ordering::SeqCst);
    if tbc_msg != 0 && msg == tbc_msg {
        with_inner(app, |inner| {
            inner.added = false;
            inner.add_or_update();
        });
    } else if msg == WM_COMMAND {
        let notify = ((wparam.0 >> 16) & 0xFFFF) as u32;
        if notify == THBN_CLICKED {
            let id = (wparam.0 & 0xFFFF) as u32;
            let action = match id {
                ID_PREV => Some("previous"),
                ID_PLAYPAUSE => Some("toggle"),
                ID_NEXT => Some("next"),
                _ => None,
            };
            if let Some(action) = action {
                let _ = app.emit(
                    "media-control",
                    serde_json::json!({ "action": action, "position": serde_json::Value::Null }),
                );
                return LRESULT(0);
            }
        }
    } else if msg == WM_SETTINGCHANGE && lparam_is(lparam, "ImmersiveColorSet") {
        let size = GetSystemMetrics(SM_CXSMICON).max(16);
        with_inner(app, |inner| {
            let light = system_uses_light_theme();
            if light != inner.light_theme {
                inner.light_theme = light;
                inner.destroy_icons();
                inner.build_icons(size);
                if inner.added {
                    inner.update();
                }
            }
        });
    }

    DefSubclassProc(hwnd, msg, wparam, lparam)
}

// Run `f` against the live controller state, if present.
unsafe fn with_inner<F: FnOnce(&mut Inner)>(app: &AppHandle, f: F) {
    if let Some(controller) = app.try_state::<ThumbbarController>() {
        if let Ok(mut guard) = controller.0.lock() {
            if let Some(inner) = guard.as_mut() {
                f(inner);
            }
        }
    }
}
