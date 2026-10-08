//! Effect value types.

/// Where an effect's shader build is. An effect draws only when `Ready`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum EffectStatus {
    /// Compiling on a worker thread; the chain passes the picture through it.
    Building,
    Ready,
    /// The build failed; the effect stays in its chain and is skipped.
    Failed {
        message: String,
    },
}
