//! 配置唯一存放点。
//!
//! 运行期配置只有这一份内存副本，任何写入都必须经过本类型（`replace` /
//! `update`），并因此递增单调版本号。读取方拿 `Arc<Config>` 快照，通过
//! `version()` 判断是否需要重新应用，而不是各自持有一份可变拷贝 —— 从
//! 而消除「内存 RwLock / 磁盘 / 引擎内副本」多写者对账产生的竞态。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use crate::config::Config;

/// 内存配置副本 + 版本号。可安全地以 `Arc<ConfigStore>` 在进程内共享。
pub struct ConfigStore {
    config: RwLock<Arc<Config>>,
    version: AtomicU64,
}

impl ConfigStore {
    /// 以给定配置创建，初始版本为 1。
    pub fn new(config: Config) -> Self {
        Self {
            config: RwLock::new(Arc::new(config)),
            version: AtomicU64::new(1),
        }
    }

    /// 从磁盘加载并创建。
    pub fn load() -> Self {
        Self::new(Config::load())
    }

    /// 读取当前配置快照（`Arc` 克隆，不深拷贝 Config）。
    pub fn get(&self) -> Arc<Config> {
        self.config
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// 当前版本号（每次写入递增，可用于跳过重复应用）。
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    /// 用新配置整体替换内存副本，返回递增后的版本号。
    pub fn replace(&self, config: Config) -> u64 {
        {
            let mut guard = self.config.write().unwrap_or_else(|e| e.into_inner());
            *guard = Arc::new(config);
        }
        self.version.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// 就地进行一次修改（写锁内），返回闭包结果与递增后的版本号。
    ///
    /// 仅当存在其他 `Arc` 读者时才会真正深拷贝（`Arc::make_mut` 语义）。
    pub fn update<R>(&self, f: impl FnOnce(&mut Config) -> R) -> (R, u64) {
        let (result, version) = {
            let mut guard = self.config.write().unwrap_or_else(|e| e.into_inner());
            let result = f(Arc::make_mut(&mut guard));
            (result, self.version.fetch_add(1, Ordering::AcqRel) + 1)
        };
        (result, version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_increments_on_replace_and_update() {
        let store = ConfigStore::new(Config::default_config());
        assert_eq!(store.version(), 1);

        let v = store.replace(Config::default_config());
        assert_eq!(v, 2);
        assert_eq!(store.version(), 2);

        let (_, v) = store.update(|c| {
            c.appearance.show_candidates = false;
        });
        assert_eq!(v, 3);
        assert!(!store.get().appearance.show_candidates);
    }

    #[test]
    fn get_returns_shared_snapshot() {
        let store = ConfigStore::new(Config::default_config());
        let a = store.get();
        let b = store.get();
        // 未写入时两次读取应共享同一份，不重复分配
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn update_does_not_leak_to_previous_snapshot() {
        let store = ConfigStore::new(Config::default_config());
        let before = store.get();
        store.update(|c| {
            c.appearance.show_candidates = false;
        });
        // 旧快照保持不变，新读取拿到新值
        assert!(before.appearance.show_candidates);
        assert!(!store.get().appearance.show_candidates);
    }
}
