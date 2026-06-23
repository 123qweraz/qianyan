#[derive(Debug, Clone)]
pub enum TrayEvent {
    ToggleIme,
    ToggleEnabled,
    NextProfile,
    OpenConfig,
    OpenFeatureCenter,
    Exit,
    ReloadConfig,
    SyncStatus {
        chinese_enabled: bool,
        active_profile: String,
    },
    ShowNotification(String),
    ClearUserDict(Option<String>),
    SendKey(String),
    SetProfile(String),
    FeatureReady(u16),
}
