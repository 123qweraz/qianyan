use std::sync::OnceLock;
use windows::{core::*, Win32::Foundation::*, Win32::System::SystemServices::DLL_PROCESS_ATTACH};

mod class_factory;
mod registry;
mod text_service;
mod tsf;

use class_factory::ClassFactory;

mod constants;
pub use constants::{IME_ID, LANG_PROFILE_ID};

#[cfg(target_os = "windows")]
pub fn _is_system_dark_mode() -> bool {
    use windows::Win32::System::Registry::*;
    use windows::core::PCWSTR;

    let mut hkey = HKEY::default();
    let sub_key = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0".encode_utf16().collect::<Vec<u16>>();
    
    unsafe {
        if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(sub_key.as_ptr()), 0, KEY_READ, &mut hkey).is_ok() {
            let mut value: u32 = 0;
            let mut size = std::mem::size_of::<u32>() as u32;
            let value_name = "AppsUseLightTheme\0".encode_utf16().collect::<Vec<u16>>();
            
            let res = RegQueryValueExW(hkey, PCWSTR(value_name.as_ptr()), None, None, Some(&mut value as *mut _ as *mut u8), Some(&mut size));
            let _ = RegCloseKey(hkey);
            
            if res.is_ok() {
                return value == 0;
            }
        }
    }
    false
}

static DLL_INSTANCE: OnceLock<HINSTANCE> = OnceLock::new();

#[no_mangle]
#[allow(non_snake_case)]
unsafe extern "system" fn DllMain(
    dll_module: HINSTANCE,
    call_reason: u32,
    _reserved: *mut std::ffi::c_void,
) -> bool {
    if call_reason == DLL_PROCESS_ATTACH {
        let _ = DLL_INSTANCE.set(dll_module);
    }
    true
}

#[no_mangle]
#[allow(non_snake_case)]
pub unsafe extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut std::ffi::c_void,
) -> HRESULT {
    if *rclsid != IME_ID {
        return CLASS_E_CLASSNOTAVAILABLE;
    }

    let factory = ClassFactory::new();
    let unknown: IUnknown = factory.into();

    unknown.query(&*riid, ppv)
}

#[no_mangle]
#[allow(non_snake_case)]
pub unsafe extern "system" fn DllRegisterServer() -> HRESULT {
    if let Some(&instance) = DLL_INSTANCE.get() {
        registry::register_server(instance, &IME_ID, "Qianyan IME", None)
            .map_or_else(|e| e.code(), |_| S_OK)
    } else {
        CO_E_NOTINITIALIZED
    }
}

#[no_mangle]
#[allow(non_snake_case)]
pub unsafe extern "system" fn DllUnregisterServer() -> HRESULT {
    registry::unregister_server(&IME_ID).map_or_else(|e| e.code(), |_| S_OK)
}
