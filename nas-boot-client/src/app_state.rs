#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum AppState {
    #[default]
    Unknown,

    /// User idle or WOL disabled
    Idle,

    /// User active and waking up NAS
    WakeUp,

    /// NAS is ready
    NasReady,
}
