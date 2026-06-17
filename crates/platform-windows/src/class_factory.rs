use std::sync::atomic::{AtomicI32, Ordering};
use crate::text_service::TextService;
use windows::{core::*, Win32::Foundation::*, Win32::System::Com::*};

pub(crate) static LOCK_COUNT: AtomicI32 = AtomicI32::new(0);

#[implement(IClassFactory)]
pub struct ClassFactory {}

impl ClassFactory {
    pub fn new() -> Self {
        Self {}
    }
}

impl IClassFactory_Impl for ClassFactory {
    fn CreateInstance(
        &self,
        p_unk_outer: Option<&IUnknown>,
        riid: *const GUID,
        ppv_object: *mut *mut std::ffi::c_void,
    ) -> Result<()> {
        if ppv_object.is_null() {
            return Err(E_POINTER.into());
        }

        // ClassFactory 不支持聚合
        if p_unk_outer.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }

        // 创建我们的 TextService 实例
        let service = TextService::new();
        let unknown: IUnknown = service.into();

        // 查询请求的接口 (riid)
        unsafe {
            let hr = unknown.query(&*riid, ppv_object);
            if hr.is_err() {
                *ppv_object = std::ptr::null_mut();
            }
            hr.ok()
        }
    }

    fn LockServer(&self, f_lock: BOOL) -> Result<()> {
        if f_lock.0 != 0 {
            LOCK_COUNT.fetch_add(1, Ordering::Relaxed);
        } else {
            LOCK_COUNT.fetch_sub(1, Ordering::Relaxed);
        }
        Ok(())
    }
}

pub(crate) fn can_unload_now() -> bool {
    LOCK_COUNT.load(Ordering::Relaxed) == 0
}
