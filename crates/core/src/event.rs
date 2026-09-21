#[derive(Debug, Clone)]
pub enum TrayEvent {
    ToggleIme,
    ToggleEnabled,
    NextProfile,
    OpenConfig,
    OpenFeatureCenter,
    Exit,
    ReloadConfig,
    ReloadConfigFromWeb,
    SyncStatus {
        chinese_enabled: bool,
        active_profile: String,
    },
    ShowNotification(String),
    ClearUserDict(Option<String>),
    SendKey { key: String, action: String },
    SetProfile(String),
    FeatureReady(u16),
}
