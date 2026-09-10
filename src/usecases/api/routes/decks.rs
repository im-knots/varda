//! Deck CRUD and property routes.
//!
//! Deck UUIDs are globally unique, so routes that name an existing deck are flat
//! (`/api/decks/{deck_uuid}`). The owning channel stays in the path only for
//! creation (no deck UUID exists yet) and for reorder (the ordinals are scoped
//! to one channel).

use axum::Json;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::channel::DeckRenderFps;
use crate::engine::{CommandResult, EngineCommand};
use crate::usecases::api::{SharedState, command_response};

/// Remove every `..` component, joining what remains.
///
/// Pure: no filesystem access, so it behaves identically on every platform and
/// can be tested exhaustively without touching disk.
fn strip_parent_dirs(p: &std::path::Path) -> std::path::PathBuf {
    p.components()
        .filter(|c| !matches!(c, std::path::Component::ParentDir))
        .collect()
}

/// Resolve a caller-supplied media path.
///
/// If the path resolves on disk, `canonicalize` gives the real target. If it
/// does not, fall back to [`strip_parent_dirs`] so a request naming a missing
/// file cannot walk upward through the tree on its way to the error.
fn sanitize_path(p: &std::path::Path) -> std::path::PathBuf {
    p.canonicalize().unwrap_or_else(|_| strip_parent_dirs(p))
}

#[cfg(test)]
mod sanitize_path_tests {
    use super::{sanitize_path, strip_parent_dirs};
    use std::path::{Component, Path, PathBuf};

    fn has_parent_dir(p: &Path) -> bool {
        p.components().any(|c| matches!(c, Component::ParentDir))
    }

    // The stripping half is pure, so these assert on it directly rather than
    // going through `sanitize_path` and hoping the filesystem declines to
    // resolve the input.
    //
    // These tests used to do exactly that, via a helper that asserted its input
    // did not exist. It passed everywhere for as long as Varda's tests only ran
    // on macOS and Linux, and failed the first time they ran on Windows:
    // `nope_zzz/..` does not exist on POSIX, which resolves `..` against real
    // directories and so needs `nope_zzz` to be there, but Win32 collapses `..`
    // textually before the filesystem sees the path, leaving the current
    // directory, which certainly does exist. The precondition was never a fact
    // about the input; it was a fact about POSIX.

    #[test]
    fn strips_all_parent_dir_components() {
        let cleaned = strip_parent_dirs(Path::new("/base/foo/../bar/../../baz"));
        assert!(!has_parent_dir(&cleaned), "'..' survived: {cleaned:?}");
    }

    #[test]
    fn retains_non_traversal_components() {
        let cleaned = strip_parent_dirs(Path::new("../../secret/asset.png"));
        assert!(!has_parent_dir(&cleaned));
        assert_eq!(cleaned, PathBuf::from("secret/asset.png"));
    }

    #[test]
    fn normal_relative_path_passes_through_unchanged() {
        let original = Path::new("assets/textures/tile.png");
        assert_eq!(strip_parent_dirs(original), original);
    }

    #[test]
    fn a_trailing_parent_dir_leaves_the_preceding_component() {
        // Named for what it does. The old name claimed "reduces to empty" while
        // asserting the opposite.
        assert_eq!(strip_parent_dirs(Path::new("foo/..")), PathBuf::from("foo"));
    }

    #[test]
    fn nothing_but_parent_dirs_reduces_to_empty() {
        assert_eq!(strip_parent_dirs(Path::new("../../..")), PathBuf::new());
    }

    #[test]
    fn stripping_is_not_resolving() {
        // Worth pinning down, because the difference is easy to misread.
        // Dropping `..` does not cancel the component before it: `/etc/../var`
        // becomes `/etc/var`, a third path that is neither the input nor its
        // normalized form. That is what "strip" means here, and it is the safe
        // direction (it can only ever move *down* the tree), but it does mean a
        // caller's legitimate relative path can come back naming something else
        // when the original does not resolve.
        let cleaned = strip_parent_dirs(Path::new("/etc/../var/log"));
        assert_eq!(cleaned, PathBuf::from("/etc/var/log"));

        // The root itself has to survive, or an absolute path would quietly
        // turn into a relative one.
        assert!(cleaned.is_absolute(), "lost the root: {cleaned:?}");
    }

    // And one test for the branch that does touch disk, on a path built to
    // exist rather than one hoped not to.

    #[test]
    fn an_existing_path_is_resolved_rather_than_stripped() {
        // The input has to be one where resolving and stripping *disagree*, or
        // the test cannot tell whether `canonicalize` ran at all. A tempdir path
        // is already canonical, so `<tmp>/clip.png` passes either way.
        //
        // `<tmp>/sub/../clip.png` does not: resolving gives `<tmp>/clip.png`,
        // which is the real file, while stripping gives `<tmp>/sub/clip.png`,
        // which is not.
        let dir = tempfile::tempdir().expect("tempdir");
        // macOS puts tempdirs under a symlinked `/var`, so canonicalize the root
        // too and compare like with like.
        let root = dir.path().canonicalize().expect("canonicalize tempdir");
        std::fs::create_dir(root.join("sub")).expect("create sub");
        let real = root.join("clip.png");
        std::fs::write(&real, b"x").expect("write");

        let resolved = sanitize_path(&root.join("sub").join("..").join("clip.png"));
        assert_eq!(resolved, real, "not resolved to the real file");
        assert!(!has_parent_dir(&resolved));
        assert_ne!(
            resolved,
            root.join("sub").join("clip.png"),
            "took the stripping branch on a path that resolves"
        );
    }

    #[test]
    fn a_missing_path_falls_back_to_stripping() {
        let dir = tempfile::tempdir().expect("tempdir");
        // `<tmp>/sub/../gone.png`: `sub` does not exist, so POSIX cannot resolve
        // it; Win32 collapses it to `<tmp>/gone.png`, which does not exist
        // either. Neither platform can canonicalize this, so both take the
        // stripping branch.
        let missing = dir.path().join("sub").join("..").join("gone.png");

        let resolved = sanitize_path(&missing);
        assert!(!has_parent_dir(&resolved), "'..' survived: {resolved:?}");
        assert_eq!(
            resolved.file_name(),
            Some(std::ffi::OsStr::new("gone.png")),
            "lost the filename: {resolved:?}"
        );
    }
}

#[derive(Deserialize, ToSchema)]
pub struct AddShaderDeckBody {
    /// Name of the shader to load into the new deck.
    pub shader_name: String,
}

#[derive(Deserialize, ToSchema)]
pub struct DeckOpacityBody {
    /// Opacity value from 0.0 (transparent) to 1.0 (opaque).
    pub opacity: f32,
}

#[derive(Deserialize, ToSchema)]
pub struct DeckBlendModeBody {
    /// Blend mode for compositing this deck.
    pub mode: crate::engine::BlendMode,
}

#[derive(Deserialize, ToSchema)]
pub struct DeckBoolBody {
    /// Boolean toggle value.
    pub value: bool,
}

#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/shader", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = AddShaderDeckBody, responses((status = 200, body = CommandResult), (status = 404, description = "Channel not found")), tag = "Decks")]
pub async fn add_shader_deck(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(body): Json<AddShaderDeckBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::AddDeck {
            channel_uuid,
            shader_name: body.shader_name,
        })
        .await
    {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(delete, path = "/api/decks/{deck_uuid}", params(("deck_uuid" = String, Path, description = "Deck UUID")), responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")), tag = "Decks")]
pub async fn remove_deck(
    State(state): State<SharedState>,
    Path(deck_uuid): Path<String>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::RemoveDeck { deck_uuid })
        .await
    {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/decks/{deck_uuid}/opacity", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = DeckOpacityBody, responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")), tag = "Decks")]
pub async fn set_opacity(
    State(state): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(body): Json<DeckOpacityBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::SetDeckOpacity {
            deck_uuid,
            opacity: body.opacity,
        })
        .await
    {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/decks/{deck_uuid}/blend-mode", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = DeckBlendModeBody, responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")), tag = "Decks")]
pub async fn set_blend_mode(
    State(state): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(body): Json<DeckBlendModeBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::SetDeckBlendMode {
            deck_uuid,
            mode: body.mode,
        })
        .await
    {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/decks/{deck_uuid}/solo", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = DeckBoolBody, responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")), tag = "Decks")]
pub async fn set_solo(
    State(state): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(body): Json<DeckBoolBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::SetDeckSolo {
            deck_uuid,
            solo: body.value,
        })
        .await
    {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/decks/{deck_uuid}/mute", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = DeckBoolBody, responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")), tag = "Decks")]
pub async fn set_mute(
    State(state): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(body): Json<DeckBoolBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::SetDeckMute {
            deck_uuid,
            mute: body.value,
        })
        .await
    {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct AddImageDeckBody {
    /// File path to the image asset.
    #[schema(value_type = String)]
    pub path: std::path::PathBuf,
}

#[derive(Deserialize, ToSchema)]
pub struct AddVideoDeckBody {
    /// File path to the video asset.
    #[schema(value_type = String)]
    pub path: std::path::PathBuf,
}

#[derive(Deserialize, ToSchema)]
pub struct AddSolidColorDeckBody {
    /// RGBA colour as four floats in 0.0–1.0.
    pub color: [f32; 4],
}

#[derive(Deserialize, ToSchema)]
pub struct AddCameraDeckBody {
    /// Numeric identifier of the camera device.
    pub camera_id: u32,
}

#[derive(Deserialize, ToSchema)]
pub struct AddDepthSensorDeckBody {
    /// Numeric identifier of the depth sensor device.
    pub depth_sensor_id: u32,
}

/// Capture target in handle-free form. Either `{"kind":"display","name":"..."}`
/// or `{"kind":"window","app":"...","title":"..."}` — matched against the last
/// enumeration, so call `POST /api/devices/screen/scan` first if unsure.
#[derive(Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScreenCaptureTargetBody {
    Display {
        name: String,
    },
    Window {
        app: String,
        #[serde(default)]
        title: String,
    },
}

impl From<ScreenCaptureTargetBody> for crate::scene::CaptureTargetConfig {
    fn from(b: ScreenCaptureTargetBody) -> Self {
        match b {
            ScreenCaptureTargetBody::Display { name } => Self::Display { name },
            ScreenCaptureTargetBody::Window { app, title } => Self::Window { app, title },
        }
    }
}

#[derive(Deserialize, ToSchema)]
pub struct ScreenCaptureCropBody {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Deserialize, ToSchema)]
pub struct AddScreenCaptureDeckBody {
    pub target: ScreenCaptureTargetBody,
    /// Capture frames per second, 1–120. Defaults to 30.
    #[serde(default)]
    pub rate: Option<f32>,
    /// Normalized crop within the target. Omit for the full frame.
    #[serde(default)]
    pub crop: Option<ScreenCaptureCropBody>,
    #[serde(default)]
    pub show_cursor: Option<bool>,
    /// Exclude Varda's own windows. Defaults to `true` for displays (so a
    /// full-display capture is not an infinite mirror) and `false` for windows.
    #[serde(default)]
    pub exclude_varda: Option<bool>,
}

/// Which Varda-internal output a tap deck reads. See spec/program-tap.md.
#[derive(Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TapSourceBody {
    MasterProgram,
    Channel { uuid: String },
}

impl From<TapSourceBody> for crate::scene::TapSourceConfig {
    fn from(b: TapSourceBody) -> Self {
        match b {
            TapSourceBody::MasterProgram => Self::MasterProgram,
            TapSourceBody::Channel { uuid } => Self::Channel { uuid },
        }
    }
}

#[derive(Deserialize, ToSchema)]
pub struct TapSourceRequestBody {
    pub source: TapSourceBody,
}

#[derive(Deserialize, ToSchema)]
pub struct MoveDeckBody {
    /// UUID of the channel to move the deck into.
    pub dst_channel_uuid: String,
}

#[derive(Deserialize, ToSchema)]
pub struct ReorderDeckBody {
    /// Current position of the deck within its channel.
    pub from_idx: usize,
    /// Target position of the deck within its channel.
    pub to_idx: usize,
}

#[derive(Deserialize, ToSchema)]
pub struct DeckScalingModeBody {
    /// How the deck content is scaled to fit the output.
    pub mode: crate::internal::deck::ScalingMode,
}

#[derive(Deserialize, ToSchema)]
pub struct SetTransitionBody {
    /// Shader name for the transition, or null to clear.
    pub shader_name: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct SetParamBody {
    /// Slash-separated path identifying the parameter, e.g.
    /// `deck/<deck_uuid>/param/<name>`,
    /// `deck/<deck_uuid>/effect/<fx_uuid>/param/<name>`, or `deck/<uuid>/opacity`.
    pub path: String,
    /// New value, interpreted against the parameter's declared ISF type.
    ///
    /// A `float` parameter takes a **normalized 0.0-1.0 fraction**, scaled to the
    /// parameter's declared MIN/MAX range: on a param with `MIN 0.0, MAX 3.0`,
    /// send `0.5` to get `1.5`. This matches the MIDI, OSC, and fader paths, and
    /// is why `GET /api/scene` reports a raw value while this route takes a
    /// normalized one. A `long` takes a **discrete choice index** (`2` selects the
    /// third entry in its VALUES list, not 2% of a range), a `bool` takes a flag,
    /// and `color` / `point2D` take their full arrays.
    pub value: crate::internal::params::ParamValue,
}

#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/image", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = AddImageDeckBody, responses((status = 200, body = CommandResult), (status = 404, description = "Channel not found")), tag = "Decks")]
pub async fn add_image_deck(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(body): Json<AddImageDeckBody>,
) -> impl IntoResponse {
    let path = sanitize_path(&body.path);
    match state
        .send_command(EngineCommand::AddImageDeck { channel_uuid, path })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/video", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = AddVideoDeckBody, responses((status = 200, body = CommandResult), (status = 404, description = "Channel not found")), tag = "Decks")]
pub async fn add_video_deck(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(body): Json<AddVideoDeckBody>,
) -> impl IntoResponse {
    let path = sanitize_path(&body.path);
    match state
        .send_command(EngineCommand::AddVideoDeck { channel_uuid, path })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/solid", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = AddSolidColorDeckBody, responses((status = 200, body = CommandResult), (status = 404, description = "Channel not found")), tag = "Decks")]
pub async fn add_solid_color_deck(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(body): Json<AddSolidColorDeckBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::AddSolidColorDeck {
            channel_uuid,
            color: body.color,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/camera", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = AddCameraDeckBody, responses((status = 200, body = CommandResult), (status = 404, description = "Channel not found")), tag = "Decks")]
pub async fn add_camera_deck(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(body): Json<AddCameraDeckBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::AddCameraDeck {
            channel_uuid,
            camera_id: body.camera_id,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/depth", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = AddDepthSensorDeckBody, responses((status = 200, body = CommandResult), (status = 404, description = "Channel not found")), tag = "Depth Sensors")]
pub async fn add_depth_sensor_deck(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(body): Json<AddDepthSensorDeckBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::AddDepthSensorDeck {
            channel_uuid,
            depth_sensor_id: body.depth_sensor_id,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/screen", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = AddScreenCaptureDeckBody, responses((status = 200, body = CommandResult), (status = 404, description = "Channel or capture target not found")), tag = "Screen Capture")]
pub async fn add_screen_capture_deck(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(body): Json<AddScreenCaptureDeckBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::AddScreenCaptureDeck {
            channel_uuid,
            target: body.target.into(),
            rate: body.rate,
            crop: body.crop.map(|c| crate::scene::CaptureCropConfig {
                x: c.x,
                y: c.y,
                w: c.w,
                h: c.h,
            }),
            show_cursor: body.show_cursor,
            exclude_varda: body.exclude_varda,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/tap", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = TapSourceRequestBody, responses((status = 200, body = CommandResult), (status = 404, description = "Channel not found")), tag = "Decks")]
pub async fn add_tap_deck(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(body): Json<TapSourceRequestBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::AddTapDeck {
            channel_uuid,
            source: body.source.into(),
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/decks/{deck_uuid}/tap/source", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = TapSourceRequestBody, responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found or is not a tap")), tag = "Decks")]
pub async fn set_tap_source(
    State(state): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(body): Json<TapSourceRequestBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::SetTapSource {
            deck_uuid,
            source: body.source.into(),
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/decks/{deck_uuid}/move", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = MoveDeckBody, responses((status = 200, body = CommandResult), (status = 404, description = "Deck or destination channel not found")), tag = "Decks")]
pub async fn move_deck(
    State(state): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(body): Json<MoveDeckBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::MoveDeck {
            deck_uuid,
            dst_channel_uuid: body.dst_channel_uuid,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/channels/{channel_uuid}/decks/reorder", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = ReorderDeckBody, responses((status = 200, body = CommandResult), (status = 404, description = "Channel not found")), tag = "Decks")]
pub async fn reorder_deck(
    State(state): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(body): Json<ReorderDeckBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::ReorderDeck {
            channel_uuid,
            from_idx: body.from_idx,
            to_idx: body.to_idx,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct DeckRenderFpsBody {
    pub render_fps: DeckRenderFps,
}

#[utoipa::path(put, path = "/api/decks/{deck_uuid}/render-fps", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = DeckRenderFpsBody, responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")), tag = "Decks")]
pub async fn set_render_fps(
    State(state): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(body): Json<DeckRenderFpsBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::SetDeckRenderFps {
            deck_uuid,
            render_fps: body.render_fps,
        })
        .await
    {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/decks/{deck_uuid}/transparent", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = DeckBoolBody, responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")), tag = "Decks")]
pub async fn set_transparent(
    State(state): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(body): Json<DeckBoolBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::SetDeckTransparent {
            deck_uuid,
            transparent: body.value,
        })
        .await
    {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/decks/{deck_uuid}/scaling-mode", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = DeckScalingModeBody, responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")), tag = "Decks")]
pub async fn set_scaling_mode(
    State(state): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(body): Json<DeckScalingModeBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::SetDeckScalingMode {
            deck_uuid,
            mode: body.mode,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/mixer/transition", request_body = SetTransitionBody, responses((status = 200, body = CommandResult)), tag = "Mixer")]
pub async fn set_transition(
    State(state): State<SharedState>,
    Json(body): Json<SetTransitionBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::SetTransition {
            shader_name: body.shader_name,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(put, path = "/api/params", request_body = SetParamBody, responses((status = 200, body = CommandResult)), tag = "Params")]
pub async fn set_param(
    State(state): State<SharedState>,
    Json(body): Json<SetParamBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::SetParam {
            path: body.path,
            value: body.value,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

/// Applies any `EngineCommand` sent as JSON and returns its `CommandResult`.
///
/// The body is an externally-tagged `EngineCommand`, documented as unconstrained
/// JSON: the enum has no `ToSchema` derive, and a hand-written approximation of
/// several hundred variants would drift from the real vocabulary. Use the typed
/// routes for a documented body.
#[utoipa::path(post, path = "/api/command",
    request_body = serde_json::Value,
    responses((status = 200, body = CommandResult)),
    tag = "System")]
pub async fn generic_command(
    State(state): State<SharedState>,
    Json(cmd): Json<EngineCommand>,
) -> impl IntoResponse {
    match state.send_command(cmd).await {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

// ── Video Playback ─────────────────────────────────────────────────

#[utoipa::path(post, path = "/api/decks/{deck_uuid}/video/toggle-play", params(("deck_uuid" = String, Path, description = "Deck UUID")), responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")), tag = "Video")]
pub async fn video_toggle_play(
    State(s): State<SharedState>,
    Path(deck_uuid): Path<String>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::VideoTogglePlay { deck_uuid })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct VideoSeekBody {
    /// Seek position in seconds from the start of the video.
    pub position_secs: f64,
}
#[utoipa::path(put, path = "/api/decks/{deck_uuid}/video/seek", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = VideoSeekBody, responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")), tag = "Video")]
pub async fn video_seek(
    State(s): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(b): Json<VideoSeekBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::VideoSeek {
            deck_uuid,
            position_secs: b.position_secs,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct VideoSpeedBody {
    /// Playback speed multiplier (1.0 = normal speed).
    pub speed: f64,
}
#[utoipa::path(put, path = "/api/decks/{deck_uuid}/video/speed", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = VideoSpeedBody, responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")), tag = "Video")]
pub async fn video_set_speed(
    State(s): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(b): Json<VideoSpeedBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::VideoSetSpeed {
            deck_uuid,
            speed: b.speed,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct VideoLoopModeBody {
    /// Loop behaviour for the video.
    pub mode: crate::video::LoopMode,
}
#[utoipa::path(put, path = "/api/decks/{deck_uuid}/video/loop-mode", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = VideoLoopModeBody, responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")), tag = "Video")]
pub async fn video_set_loop_mode(
    State(s): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(b): Json<VideoLoopModeBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::VideoSetLoopMode {
            deck_uuid,
            mode: b.mode,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct VideoPointBody {
    /// Time position in seconds.
    pub secs: f64,
}
#[utoipa::path(put, path = "/api/decks/{deck_uuid}/video/in-point", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = VideoPointBody, responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")), tag = "Video")]
pub async fn video_set_in_point(
    State(s): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(b): Json<VideoPointBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::VideoSetInPoint {
            deck_uuid,
            secs: b.secs,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/decks/{deck_uuid}/video/out-point", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = VideoPointBody, responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")), tag = "Video")]
pub async fn video_set_out_point(
    State(s): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(b): Json<VideoPointBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::VideoSetOutPoint {
            deck_uuid,
            secs: b.secs,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(delete, path = "/api/decks/{deck_uuid}/video/in-out-points", params(("deck_uuid" = String, Path, description = "Deck UUID")), responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")), tag = "Video")]
pub async fn video_clear_in_out(
    State(s): State<SharedState>,
    Path(deck_uuid): Path<String>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::VideoClearInOutPoints { deck_uuid })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct VideoTransportSyncBody {
    pub mode: crate::video::TransportSyncMode,
    #[serde(default)]
    pub offset: f64,
    #[serde(default)]
    pub delay_frames: i32,
}
#[utoipa::path(
    put,
    path = "/api/decks/{deck_uuid}/video/transport-sync",
    params(("deck_uuid" = String, Path, description = "Deck UUID")),
    request_body = VideoTransportSyncBody,
    responses(
        (status = 200, body = CommandResult),
        (status = 400, description = "Not a video deck"),
        (status = 404, description = "Deck not found")
    ),
    tag = "Video"
)]
pub async fn video_set_transport_sync(
    State(s): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(b): Json<VideoTransportSyncBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::VideoSetTransportSync {
            deck_uuid,
            sync: crate::video::DeckTransportSync {
                mode: b.mode,
                offset: b.offset,
                delay_frames: b.delay_frames,
            },
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

// ── Auto-Transitions ───────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct AutoTransBoolBody {
    /// Boolean toggle value.
    pub value: bool,
}
#[utoipa::path(put, path = "/api/decks/{deck_uuid}/auto-transition/enabled", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = AutoTransBoolBody, responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")), tag = "Auto Transitions")]
pub async fn set_auto_transition_enabled(
    State(s): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(b): Json<AutoTransBoolBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetAutoTransitionEnabled {
            deck_uuid,
            enabled: b.value,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/decks/{deck_uuid}/auto-transition/trigger", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = AutoTransBoolBody, responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")), tag = "Auto Transitions")]
pub async fn set_auto_transition_trigger(
    State(s): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(b): Json<AutoTransBoolBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetAutoTransitionTrigger {
            deck_uuid,
            clip_end: b.value,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct DurationBody {
    /// Numeric duration value.
    pub value: f64,
    /// Unit of the duration (seconds or beats).
    pub unit: crate::channel::DurationUnit,
}
#[utoipa::path(put, path = "/api/decks/{deck_uuid}/auto-transition/play-duration", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = DurationBody, responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")), tag = "Auto Transitions")]
pub async fn set_auto_transition_play_duration(
    State(s): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(b): Json<DurationBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetAutoTransitionPlayDuration {
            deck_uuid,
            value: b.value,
            unit: b.unit,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/decks/{deck_uuid}/auto-transition/duration", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = DurationBody, responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")), tag = "Auto Transitions")]
pub async fn set_auto_transition_duration(
    State(s): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(b): Json<DurationBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetAutoTransitionDuration {
            deck_uuid,
            value: b.value,
            unit: b.unit,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct ShaderNameBody {
    /// Shader name, or null to clear.
    pub shader_name: Option<String>,
}
#[utoipa::path(put, path = "/api/decks/{deck_uuid}/auto-transition/shader", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = ShaderNameBody, responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")), tag = "Auto Transitions")]
pub async fn set_auto_transition_shader(
    State(s): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(b): Json<ShaderNameBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetAutoTransitionShader {
            deck_uuid,
            shader_name: b.shader_name,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

// ── External I/O Sources ───────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct NdiSourceBody {
    /// Name of the NDI source to receive.
    pub source_name: String,
}
#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/ndi", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = NdiSourceBody, responses((status = 200, body = CommandResult), (status = 404, description = "Channel not found")), tag = "Decks")]
pub async fn add_ndi_deck(
    State(s): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(b): Json<NdiSourceBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::AddNdiDeck {
            channel_uuid,
            source_name: b.source_name,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct SyphonSourceBody {
    /// Name of the Syphon server to receive.
    pub server_name: String,
}
#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/syphon", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = SyphonSourceBody, responses((status = 200, body = CommandResult), (status = 404, description = "Channel not found")), tag = "Decks")]
pub async fn add_syphon_deck(
    State(s): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(b): Json<SyphonSourceBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::AddSyphonDeck {
            channel_uuid,
            server_name: b.server_name,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct SpoutSourceBody {
    /// Name of the Spout sender to receive.
    pub sender_name: String,
}
#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/spout", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = SpoutSourceBody, responses((status = 200, body = CommandResult), (status = 404, description = "Channel not found")), tag = "Decks")]
pub async fn add_spout_deck(
    State(s): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(b): Json<SpoutSourceBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::AddSpoutDeck {
            channel_uuid,
            sender_name: b.sender_name,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct SrtSourceBody {
    /// SRT stream URL.
    pub url: String,
    /// SRT connection mode (caller or listener).
    pub mode: crate::stream::SrtMode,
}
#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/srt", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = SrtSourceBody, responses((status = 200, body = CommandResult), (status = 404, description = "Channel not found")), tag = "Decks")]
pub async fn add_srt_deck(
    State(s): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(b): Json<SrtSourceBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::AddSrtDeck {
            channel_uuid,
            url: b.url,
            mode: b.mode,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct HlsSourceBody {
    /// HLS stream URL (.m3u8).
    pub url: String,
}
#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/hls", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = HlsSourceBody, responses((status = 200, body = CommandResult), (status = 404, description = "Channel not found")), tag = "Decks")]
pub async fn add_hls_deck(
    State(s): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(b): Json<HlsSourceBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::AddHlsDeck {
            channel_uuid,
            url: b.url,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct DashSourceBody {
    /// DASH stream URL (.mpd).
    pub url: String,
}
#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/dash", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = DashSourceBody, responses((status = 200, body = CommandResult), (status = 404, description = "Channel not found")), tag = "Decks")]
pub async fn add_dash_deck(
    State(s): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(b): Json<DashSourceBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::AddDashDeck {
            channel_uuid,
            url: b.url,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct RtmpSourceBody {
    /// RTMP stream URL.
    pub url: String,
    /// Connection mode (Pull or Listen).
    pub mode: crate::stream::RtmpMode,
}
#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/rtmp", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = RtmpSourceBody, responses((status = 200, body = CommandResult), (status = 404, description = "Channel not found")), tag = "Decks")]
pub async fn add_rtmp_deck(
    State(s): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(b): Json<RtmpSourceBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::AddRtmpDeck {
            channel_uuid,
            url: b.url,
            mode: b.mode,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct HtmlSourceBody {
    /// HTML content URL or local file path.
    pub url: String,
}
#[utoipa::path(post, path = "/api/channels/{channel_uuid}/decks/html", params(("channel_uuid" = String, Path, description = "Channel UUID")), request_body = HtmlSourceBody, responses((status = 200, body = CommandResult), (status = 404, description = "Channel not found")), tag = "Decks")]
pub async fn add_html_deck(
    State(s): State<SharedState>,
    Path(channel_uuid): Path<String>,
    Json(b): Json<HtmlSourceBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::AddHtmlDeck {
            channel_uuid,
            url: b.url,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[utoipa::path(post, path = "/api/decks/{deck_uuid}/html/reload", params(("deck_uuid" = String, Path, description = "Deck UUID")), responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")), tag = "Decks")]
pub async fn reload_html_deck(
    State(s): State<SharedState>,
    Path(deck_uuid): Path<String>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::ReloadHtmlDeck { deck_uuid })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

/// Body for opening/closing the interactive window of an HTML deck.
#[derive(Deserialize, ToSchema)]
pub struct HtmlInteractiveBody {
    /// True to open the interactive window, false to close it.
    pub open: bool,
}

#[utoipa::path(post, path = "/api/decks/{deck_uuid}/html/interactive", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = HtmlInteractiveBody, responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")), tag = "Decks")]
pub async fn set_html_interactive(
    State(s): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(body): Json<HtmlInteractiveBody>,
) -> impl IntoResponse {
    let cmd = if body.open {
        EngineCommand::OpenHtmlInteractive { deck_uuid }
    } else {
        EngineCommand::CloseHtmlInteractive
    };
    match s.send_command(cmd).await {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

// ── Generator parameters ──────────────────────────────────────────

#[utoipa::path(post, path = "/api/decks/{deck_uuid}/params/reset", params(("deck_uuid" = String, Path, description = "Deck UUID")), responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")), tag = "Params")]
pub async fn reset_generator_params(
    State(s): State<SharedState>,
    Path(deck_uuid): Path<String>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::ResetGeneratorParamsToDefaults { deck_uuid })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

/// Body for randomize and mutate. See /spec/parameter-exploration.md.
#[derive(Deserialize, ToSchema)]
pub struct ExploreParamsBody {
    /// Inspector group to scope to. Omit to cover every eligible parameter.
    #[serde(default)]
    pub group: Option<String>,
    /// Fraction of each parameter's range to move by. Mutate only, default 0.1.
    #[serde(default)]
    pub amount: Option<f32>,
    /// Omit for a time-derived seed. The same seed reproduces the same values.
    #[serde(default)]
    pub seed: Option<u64>,
}

impl ExploreParamsBody {
    fn seed(&self) -> u64 {
        self.seed.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos() as u64)
        })
    }
}

/// Draw a deck's generator parameters afresh from their declared ranges. A given seed always
/// produces the same values, so a look found this way can be reproduced rather than only saved.
#[utoipa::path(post, path = "/api/decks/{deck_uuid}/params/randomize", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = ExploreParamsBody, responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")), tag = "Params")]
pub async fn randomize_generator_params(
    State(s): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(body): Json<ExploreParamsBody>,
) -> impl IntoResponse {
    let cmd = EngineCommand::RandomizeGeneratorParams {
        deck_uuid,
        group: body.group.clone(),
        seed: body.seed(),
    };
    match s.send_command(cmd).await {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

/// Nudge a deck's generator parameters by `amount` as a fraction of each declared range, keeping the
/// current look. The small step of the find-then-name loop, where randomize is the large one.
#[utoipa::path(post, path = "/api/decks/{deck_uuid}/params/mutate", params(("deck_uuid" = String, Path, description = "Deck UUID")), request_body = ExploreParamsBody, responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")), tag = "Params")]
pub async fn mutate_generator_params(
    State(s): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(body): Json<ExploreParamsBody>,
) -> impl IntoResponse {
    let cmd = EngineCommand::MutateGeneratorParams {
        deck_uuid,
        group: body.group.clone(),
        amount: body.amount.unwrap_or(0.1),
        seed: body.seed(),
    };
    match s.send_command(cmd).await {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct RequestAnalyzerBody {
    /// Analyzer type to request (e.g. "`face_detect`", "brightness").
    pub analyzer_type: String,
    /// Options passed to the analyzer (optional, default empty object).
    #[serde(default)]
    pub options: serde_json::Value,
}

/// Attach an analyzer to a deck (reference-counted). Body: `{"analyzer_type", "options"}`.
#[utoipa::path(post, path = "/api/decks/{deck_uuid}/analyzers",
    params(("deck_uuid" = String, Path, description = "Deck UUID")),
    request_body = RequestAnalyzerBody,
    responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")),
    tag = "Analyzers")]
pub async fn request_analyzer(
    State(state): State<SharedState>,
    Path(deck_uuid): Path<String>,
    Json(body): Json<RequestAnalyzerBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::RequestAnalyzer {
            deck_id: deck_uuid,
            analyzer_type: body.analyzer_type,
            options: body.options,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

/// Release an analyzer; it stops when the last consumer detaches.
#[utoipa::path(delete, path = "/api/decks/{deck_uuid}/analyzers/{analyzer_type}",
    params(
        ("deck_uuid" = String, Path, description = "Deck UUID"),
        ("analyzer_type" = String, Path, description = "Analyzer type to release"),
    ),
    responses((status = 200, body = CommandResult), (status = 404, description = "Deck not found")),
    tag = "Analyzers")]
pub async fn release_analyzer(
    State(state): State<SharedState>,
    Path((deck_uuid, analyzer_type)): Path<(String, String)>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::ReleaseAnalyzer {
            deck_id: deck_uuid,
            analyzer_type,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}
