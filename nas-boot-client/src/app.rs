#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum AppState {
    #[default]
    Unknown,

    /// User is idle
    UserIdle,

    /// User is active but NAS status unknown
    UserActive,

    /// NAS is available and user is active
    NasAvailable,
}
