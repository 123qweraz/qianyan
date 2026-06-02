#[cfg(target_os = "linux")]
use ksni::menu::{MenuItem, StandardItem};
#[cfg(target_os = "linux")]
use ksni::{Handle, Tray, TrayService};
use std::sync::mpsc::Sender;

#[derive(Debug, Clone)]
pub enum TrayEvent {
    ToggleIme,
    NextProfile,
    OpenConfig,
    Exit,
    ReloadConfig,
    SyncStatus {
        chinese_enabled: bool,
        active_profile: String,
    },
    ShowNotification(String),
    ClearUserDict,
    SendKey(String), // key code like "a", "Enter", "Backspace"
    SetProfile(String),
}

pub struct TrayParams {
    pub active_profile: String,
    pub enabled_profiles: Vec<String>,
    pub tx: Sender<TrayEvent>,
}

#[cfg(target_os = "linux")]
pub struct ImeTray {
    pub chinese_enabled: bool,
    pub active_profile: String,
    pub enabled_profiles: Vec<String>,
    pub tx: Sender<TrayEvent>,
}

#[cfg(target_os = "linux")]
const ALL_PROFILES: &[(&str, &str)] = &[
    ("chinese", "中文"),
    ("english", "英文"),
    ("japanese", "日文"),
    ("stroke", "笔画"),
    ("shengpizi", "生僻字"),
    ("chinese,english,japanese", "中日英混"),
];

#[cfg(target_os = "linux")]
fn load_icon(chinese_enabled: bool) -> Vec<ksni::Icon> {
    let root = qianyan_ime_core::utils::find_project_root();
    let icon_paths = if chinese_enabled {
        ["picture/qianyan-ime_v2.png", "picture/qianyan-ime.png"]
    } else {
        ["picture/qianyan-ime_v2_en.png", "picture/qianyan-ime.png"]
    };

    for path in &icon_paths {
        let full_path = root.join(path);
        if let Ok(img) = image::open(&full_path) {
            let rgba = img.to_rgba8();
            let (width, height) = rgba.dimensions();
            let data = rgba.into_raw();
            return vec![ksni::Icon {
                width: width as i32,
                height: height as i32,
                data,
            }];
        }
    }

    vec![]
}

#[cfg(target_os = "linux")]
impl Tray for ImeTray {
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        load_icon(self.chinese_enabled)
    }

    fn title(&self) -> String {
        format!(
            "qianyan ({})",
            if self.chinese_enabled { "中" } else { "英" }
        )
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let profiles: Vec<(&str, &str)> = ALL_PROFILES.iter()
            .filter(|(id, _)| self.enabled_profiles.is_empty() || self.enabled_profiles.contains(&id.to_string()))
            .copied()
            .collect();

        use ksni::menu::RadioGroup;
        use ksni::menu::RadioItem;
        use ksni::menu::SubMenu;

        let selected_idx = profiles.iter().position(|(id, _)| self.active_profile == *id).unwrap_or(0);
        let profile_options: Vec<RadioItem> = profiles
            .iter()
            .map(|(_, label)| RadioItem {
                label: label.to_string(),
                ..Default::default()
            })
            .collect();

        let profiles_copy = profiles.clone();
        let profile_radio = RadioGroup {
            selected: selected_idx,
            options: profile_options,
            select: Box::new(move |this: &mut Self, index| {
                if let Some((id, _)) = profiles_copy.get(index) {
                    let _ = this.tx.send(TrayEvent::SetProfile(id.to_string()));
                }
            }),
        };

        vec![
            StandardItem {
                label: format!("输入法: {}", if self.chinese_enabled { "中" } else { "英" }),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.send(TrayEvent::ToggleIme);
                }),
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: "词典方案".into(),
                submenu: vec![profile_radio.into()],
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "打开管理页面".to_string(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.send(TrayEvent::OpenConfig);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "重载配置".to_string(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.send(TrayEvent::ReloadConfig);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "退出程序".to_string(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.send(TrayEvent::Exit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

#[cfg(target_os = "linux")]
pub struct LinuxTrayHandle(Handle<ImeTray>);

#[cfg(target_os = "linux")]
impl LinuxTrayHandle {
    pub fn update<F>(&self, f: F)
    where
        F: FnOnce(&mut ImeTray) + Send + 'static,
    {
        self.0.update(f);
    }
}

#[cfg(target_os = "linux")]
pub fn start_tray(params: TrayParams) -> LinuxTrayHandle {
    println!("[Tray] 正在启动 Linux 系统托盘...");
    let tray = ImeTray {
        chinese_enabled: true,
        active_profile: params.active_profile,
        enabled_profiles: params.enabled_profiles,
        tx: params.tx,
    };
    let service = TrayService::new(tray);
    let handle = service.handle();
    service.spawn();
    println!("[Tray] Linux 系统托盘已启动。");
    LinuxTrayHandle(handle)
}

#[cfg(target_os = "windows")]
use std::sync::{Arc, Mutex, OnceLock};
#[cfg(target_os = "windows")]
use windows::{
    core::*, Win32::Foundation::*, Win32::UI::Shell::*, Win32::UI::WindowsAndMessaging::*,
};

#[cfg(target_os = "windows")]
const WM_TRAYICON: u32 = WM_USER + 100;
#[cfg(target_os = "windows")]
const TRAY_ICON_ID: u32 = 1;

#[cfg(target_os = "windows")]
pub struct ImeTrayStub {
    pub chinese_enabled: bool,
    pub active_profile: String,
    pub enabled_profiles: Vec<String>,
}

#[cfg(target_os = "windows")]
static TRAY_STATE: OnceLock<Arc<Mutex<ImeTrayStub>>> = OnceLock::new();
#[cfg(target_os = "windows")]
static TRAY_TX: OnceLock<Sender<TrayEvent>> = OnceLock::new();
#[cfg(target_os = "windows")]
static TRAY_HWND: OnceLock<HWND> = OnceLock::new();
#[cfg(target_os = "windows")]
static TRAY_HICON_ZH: OnceLock<HICON> = OnceLock::new();
#[cfg(target_os = "windows")]
static TRAY_HICON_EN: OnceLock<HICON> = OnceLock::new();
#[cfg(target_os = "windows")]
static TRAY_HICON_DEF: OnceLock<HICON> = OnceLock::new();

#[cfg(target_os = "windows")]
fn load_icon_win(path: &std::path::Path) -> Result<HICON, ()> {
    let abs = path.to_string_lossy().to_string() + "\0";
    let wide: Vec<u16> = abs.encode_utf16().collect();
    unsafe {
        match LoadImageW(
            None,
            PCWSTR(wide.as_ptr()),
            IMAGE_ICON,
            0, 0,
            LR_LOADFROMFILE | LR_DEFAULTSIZE,
        ) {
            Ok(h) => Ok(HICON(h.0)),
            Err(_) => Err(()),
        }
    }
}

#[cfg(target_os = "windows")]
fn make_wide_tip(s: &str) -> [u16; 128] {
    let mut buf = [0u16; 128];
    for (i, c) in s.encode_utf16().take(127).enumerate() {
        buf[i] = c;
    }
    buf
}

#[cfg(target_os = "windows")]
pub struct WindowsTrayHandle(Arc<Mutex<ImeTrayStub>>);

#[cfg(target_os = "windows")]
impl WindowsTrayHandle {
    pub fn update<F>(&self, f: F)
    where
        F: FnOnce(&mut ImeTrayStub),
    {
        if let Ok(mut state) = self.0.lock() {
            let old_cn = state.chinese_enabled;
            f(&mut *state);
            if state.chinese_enabled != old_cn {
                self.refresh_icon();
            }
        }
    }

    fn refresh_icon(&self) {
        let (hwnd, icon, tip_text) = match (TRAY_HWND.get(), self.0.lock().ok()) {
            (Some(h), Ok(ref s)) => {
                let icon = if s.chinese_enabled {
                    *TRAY_HICON_ZH.get().unwrap_or_else(|| TRAY_HICON_DEF.get().unwrap())
                } else {
                    *TRAY_HICON_EN.get().unwrap_or_else(|| TRAY_HICON_DEF.get().unwrap())
                };
                let tip = if s.chinese_enabled { "千言输入法 (中)" } else { "千言输入法 (英)" };
                (*h, icon, make_wide_tip(tip))
            }
            _ => return,
        };
        let nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_ICON_ID,
            uFlags: NIF_ICON | NIF_TIP,
            hIcon: icon,
            szTip: tip_text,
            ..Default::default()
        };
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid); }
    }
}

#[cfg(target_os = "windows")]
pub fn start_tray(params: TrayParams) -> WindowsTrayHandle {
    let state = Arc::new(Mutex::new(ImeTrayStub {
        chinese_enabled: true,
        active_profile: params.active_profile,
        enabled_profiles: params.enabled_profiles,
    }));

    TRAY_STATE.set(state.clone()).ok();
    TRAY_TX.set(params.tx).ok();

    let root = qianyan_ime_core::utils::find_project_root();

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // Pre-load icons
        let h_icon_zh = load_icon_win(&root.join("picture/zh.ico")).unwrap_or_else(|_| unsafe { LoadIconW(None, IDI_APPLICATION).unwrap_or_default() });
        let h_icon_en = load_icon_win(&root.join("picture/en.ico")).unwrap_or(h_icon_zh);
        let h_icon_def = load_icon_win(&root.join("picture/qianyan-ime_v2.ico")).unwrap_or(h_icon_zh);
        TRAY_HICON_ZH.set(h_icon_zh).ok();
        TRAY_HICON_EN.set(h_icon_en).ok();
        TRAY_HICON_DEF.set(h_icon_def).ok();

        unsafe {
            let instance = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default();
            let class_name = "QianyanIMETrayClass\0".encode_utf16().collect::<Vec<u16>>();
            let wc = WNDCLASSW {
                hInstance: instance.into(),
                lpszClassName: PCWSTR(class_name.as_ptr()),
                lpfnWndProc: Some(tray_wnd_proc),
                ..Default::default()
            };
            RegisterClassW(&wc);

            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                PCWSTR(class_name.as_ptr()),
                PCWSTR(std::ptr::null()),
                WS_POPUP,
                0, 0, 0, 0,
                None, None, instance, None,
            );
            TRAY_HWND.set(hwnd).ok();

            let icon = h_icon_zh; // start with Chinese icon
            let tip = make_wide_tip("千言输入法 (中)");
            let nid = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: hwnd,
                uID: TRAY_ICON_ID,
                uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
                uCallbackMessage: WM_TRAYICON,
                hIcon: icon,
                szTip: tip,
                ..Default::default()
            };

            if Shell_NotifyIconW(NIM_ADD, &nid).as_bool() {
                println!("[Tray] 系统托盘初始化成功。");
            }
            let _ = tx.send(hwnd);

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            Shell_NotifyIconW(NIM_DELETE, &nid);
        }
    });

    let _hwnd = rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap_or(HWND(0));
    WindowsTrayHandle(state)
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn tray_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TRAYICON => {
            if lparam.0 as u32 == WM_RBUTTONUP {
                let mut pt = POINT::default();
                let _ = GetCursorPos(&mut pt);

                if let Some(state_arc) = TRAY_STATE.get() {
                    if let Ok(state) = state_arc.lock() {
                        let h_menu = CreatePopupMenu().expect("Failed to create popup menu");

                        let toggle_label = format!(
                            "输入法: {}",
                            if state.chinese_enabled { "中" } else { "英" }
                        );
                        let mut toggle_w: Vec<u16> = toggle_label.encode_utf16().collect();
                        toggle_w.push(0);
                        let _ = AppendMenuW(h_menu, MF_STRING, 1001, PCWSTR(toggle_w.as_ptr()));

                        let h_profile_menu = CreatePopupMenu().expect("Failed to create profile menu");
                        let all_profiles = vec![
                            ("chinese", "中文"),
                            ("english", "英文"),
                            ("japanese", "日文"),
                            ("stroke", "笔画"),
                            ("shengpizi", "生僻字"),
                            ("chinese,english,japanese", "中日英混"),
                        ];
                        let profiles: Vec<&(&str, &str)> = all_profiles.iter()
                            .filter(|(id, _)| state.enabled_profiles.is_empty() || state.enabled_profiles.contains(&id.to_string()))
                            .collect();

                        for (i, (id, label)) in profiles.iter().enumerate() {
                            let mut flags = MF_STRING;
                            if state.active_profile == *id { flags |= MF_CHECKED; }
                            let mut label_w: Vec<u16> = label.encode_utf16().collect();
                            label_w.push(0);
                            let _ = AppendMenuW(h_profile_menu, flags, (2000 + i) as u32, PCWSTR(label_w.as_ptr()));
                        }

                        let profile_zh = match state.active_profile.as_str() {
                            "chinese" => "中文", "english" => "英文",
                            "japanese" => "日文", "stroke" => "笔画",
                            "shengpizi" => "生僻字",
                            "chinese,english,japanese" => "中日英混",
                            other => other,
                        };
                        let profile_label = format!("词典方案: {}", profile_zh);
                        let mut profile_w: Vec<u16> = profile_label.encode_utf16().collect();
                        profile_w.push(0);
                        let _ = AppendMenuW(h_menu, MF_POPUP, h_profile_menu.0 as usize, PCWSTR(profile_w.as_ptr()));

                        let _ = AppendMenuW(h_menu, MF_SEPARATOR, 0, None);
                        let _ = AppendMenuW(h_menu, MF_STRING, 1011, windows::core::w!("打开管理页面"));
                        let _ = AppendMenuW(h_menu, MF_STRING, 1012, windows::core::w!("重载配置"));
                        let _ = AppendMenuW(h_menu, MF_SEPARATOR, 0, None);
                        let _ = AppendMenuW(h_menu, MF_STRING, 1014, windows::core::w!("退出程序"));

                        let _ = SetForegroundWindow(hwnd);
                        let _ = TrackPopupMenu(h_menu, TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, None);
                        let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
                        let _ = DestroyMenu(h_menu);
                    }
                }
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = wparam.0 as u32;
            if let Some(tx) = TRAY_TX.get() {
                match id {
                    1001 => { let _ = tx.send(TrayEvent::ToggleIme); }
                    2000..=2005 => {
                        let profiles = vec!["chinese", "english", "japanese", "stroke", "shengpizi", "chinese,english,japanese"];
                        if let Some(profile) = profiles.get(id as usize - 2000) {
                            let _ = tx.send(TrayEvent::SetProfile(profile.to_string()));
                        }
                    }
                    1011 => { let _ = tx.send(TrayEvent::OpenConfig); }
                    1012 => { let _ = tx.send(TrayEvent::ReloadConfig); }
                    1014 => { let _ = tx.send(TrayEvent::Exit); }
                    _ => {}
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
