use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WakeMode {
    /// NAS won't be woken up or kept on
    Off,

    /// NAS will be woken up on user activity and kept on unless user is idle
    #[default]
    Auto,

    /// NAS will be kept on regardless of user activity
    AlwaysOn,
}
