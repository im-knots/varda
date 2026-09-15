//! DMX lighting patch routes: POST/PATCH/DELETE /api/lighting/*
//!
//! The structural half of lighting control. Live fixture values ride the parameter router
//! (`fixture/<uuid>/<role>`), which already gives MIDI, OSC, and automation parity; these routes
//! exist so an installation operator can repatch a rig headless, with no window to click in.
//! See /spec/lighting-routing.md § API Parity.

use axum::Json;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::engine::{CommandResult, EngineCommand};
use crate::usecases::api::{SharedState, command_response};

/// Dispatch a command and render its result, the same way every other write route does.
async fn dispatch(state: &SharedState, cmd: EngineCommand) -> axum::response::Response {
    match state.send_command(cmd).await {
        Ok(result) => command_response(result),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

/// Body of `POST /api/lighting/fixtures`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AddFixtureRequest {
    pub name: String,
    /// Profile reference, `vendor/model`, as laid out in the fixture library.
    pub profile: String,
    /// Which DMX personality of the profile to patch.
    pub mode: String,
    #[serde(default = "default_universe")]
    pub universe: u16,
    /// 1-based DMX start address, as printed on the fixture's own display.
    #[serde(default = "default_address")]
    pub address: u16,
}

fn default_universe() -> u16 {
    1
}
fn default_address() -> u16 {
    1
}

/// Body of `PATCH /api/lighting/fixtures/{uuid}`. Omitted fields are left unchanged.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateFixtureRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub universe: Option<u16>,
    #[serde(default)]
    pub address: Option<u16>,
    #[serde(default)]
    pub invert_pan: Option<bool>,
    #[serde(default)]
    pub invert_tilt: Option<bool>,
    #[serde(default)]
    pub swap_pan_tilt: Option<bool>,
}

/// Body of `POST /api/lighting/blackout` and `POST /api/lighting/enabled`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct BoolRequest {
    pub value: bool,
}

/// Patch a fixture into the rig.
#[utoipa::path(post, path = "/api/lighting/fixtures",
    request_body = AddFixtureRequest,
    responses(
        (status = 200, description = "Fixture patched; returns its UUID", body = CommandResult),
        (status = 400, description = "Patch rejected: overlapping address, unknown profile or mode, or out of range"),
        (status = 503, description = "Engine not yet initialized")),
    tag = "Lighting")]
pub async fn add_fixture(
    State(state): State<SharedState>,
    Json(req): Json<AddFixtureRequest>,
) -> impl IntoResponse {
    dispatch(
        &state,
        EngineCommand::AddFixture {
            name: req.name,
            profile: req.profile,
            mode: req.mode,
            universe: req.universe,
            address: req.address,
        },
    )
    .await
}

/// Repatch a fixture. Fields omitted from the body are left unchanged.
#[utoipa::path(patch, path = "/api/lighting/fixtures/{uuid}",
    params(("uuid" = String, Path, description = "Fixture UUID")),
    request_body = UpdateFixtureRequest,
    responses(
        (status = 200, description = "Fixture repatched", body = CommandResult),
        (status = 400, description = "Patch rejected"),
        (status = 404, description = "No such fixture"),
        (status = 503, description = "Engine not yet initialized")),
    tag = "Lighting")]
pub async fn update_fixture(
    State(state): State<SharedState>,
    Path(uuid): Path<String>,
    Json(req): Json<UpdateFixtureRequest>,
) -> impl IntoResponse {
    dispatch(
        &state,
        EngineCommand::UpdateFixture {
            uuid,
            name: req.name,
            profile: req.profile,
            mode: req.mode,
            universe: req.universe,
            address: req.address,
            invert_pan: req.invert_pan,
            invert_tilt: req.invert_tilt,
            swap_pan_tilt: req.swap_pan_tilt,
        },
    )
    .await
}

/// Remove a fixture from the rig.
#[utoipa::path(delete, path = "/api/lighting/fixtures/{uuid}",
    params(("uuid" = String, Path, description = "Fixture UUID")),
    responses(
        (status = 200, description = "Fixture removed", body = CommandResult),
        (status = 404, description = "No such fixture"),
        (status = 503, description = "Engine not yet initialized")),
    tag = "Lighting")]
pub async fn remove_fixture(
    State(state): State<SharedState>,
    Path(uuid): Path<String>,
) -> impl IntoResponse {
    dispatch(&state, EngineCommand::RemoveFixture { uuid }).await
}

/// Set the latching lighting blackout.
#[utoipa::path(post, path = "/api/lighting/blackout",
    request_body = BoolRequest,
    responses(
        (status = 200, description = "Blackout set", body = CommandResult),
        (status = 503, description = "Engine not yet initialized")),
    tag = "Lighting")]
pub async fn set_blackout(
    State(state): State<SharedState>,
    Json(req): Json<BoolRequest>,
) -> impl IntoResponse {
    dispatch(&state, EngineCommand::SetLightingBlackout(req.value)).await
}

/// Body of `POST /api/lighting/looks`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AddLookRequest {
    pub name: String,
}

/// Body of `PUT /api/lighting/looks/{uuid}/value`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetLookValueRequest {
    /// `group` or `fixture`.
    pub target_kind: String,
    pub target: String,
    pub role: String,
    /// A literal value. Omit when `palette` is given.
    #[serde(default)]
    pub value: Option<f32>,
    /// A palette UUID to reference instead of a literal.
    #[serde(default)]
    pub palette: Option<String>,
}

/// Body of `POST /api/lighting/decks`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AddLightingDeckRequest {
    pub channel: String,
    pub look: String,
}

/// Body of `PATCH /api/lighting/decks/{uuid}`. Omitted fields are unchanged.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateLightingDeckRequest {
    #[serde(default)]
    pub level: Option<f32>,
    #[serde(default)]
    pub blend: Option<crate::dmx::LightingBlend>,
    #[serde(default)]
    pub mute: Option<bool>,
    #[serde(default)]
    pub solo: Option<bool>,
    #[serde(default)]
    pub independent: Option<bool>,
    #[serde(default)]
    pub ltp_transition: Option<crate::dmx::LtpTransition>,
    /// Move the deck to this channel.
    #[serde(default)]
    pub channel: Option<String>,
}

/// Body of `POST /api/lighting/palettes`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AddPaletteRequest {
    pub name: String,
    /// `color`, `beam` or `position`. Determines which file the palette is saved in.
    pub kind: String,
}

/// Body of `PUT /api/lighting/palettes/{uuid}/value`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetPaletteValueRequest {
    pub role: String,
    pub value: f32,
    /// Store as an override for this fixture rather than as the role-space default.
    #[serde(default)]
    pub fixture: Option<String>,
}

/// Body of `POST /api/lighting/master`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct FloatRequest {
    pub value: f32,
}

/// Create a look.
#[utoipa::path(post, path = "/api/lighting/looks", request_body = AddLookRequest,
    responses((status = 200, description = "Look created; returns its UUID", body = CommandResult),
              (status = 503, description = "Engine not yet initialized")),
    tag = "Lighting")]
pub async fn add_look(
    State(state): State<SharedState>,
    Json(req): Json<AddLookRequest>,
) -> impl IntoResponse {
    dispatch(&state, EngineCommand::AddLook { name: req.name }).await
}

/// Delete a look, and any lighting decks playing it.
#[utoipa::path(delete, path = "/api/lighting/looks/{uuid}",
    params(("uuid" = String, Path, description = "Look UUID")),
    responses((status = 200, description = "Look removed", body = CommandResult),
              (status = 404, description = "No such look")),
    tag = "Lighting")]
pub async fn remove_look(
    State(state): State<SharedState>,
    Path(uuid): Path<String>,
) -> impl IntoResponse {
    dispatch(&state, EngineCommand::RemoveLook { uuid }).await
}

/// Set one role of one target inside a look, as a literal or a palette reference.
#[utoipa::path(put, path = "/api/lighting/looks/{uuid}/value",
    params(("uuid" = String, Path, description = "Look UUID")),
    request_body = SetLookValueRequest,
    responses((status = 200, description = "Value stored", body = CommandResult),
              (status = 400, description = "Bad target, role, or neither value nor palette given"),
              (status = 404, description = "No such look")),
    tag = "Lighting")]
pub async fn set_look_value(
    State(state): State<SharedState>,
    Path(uuid): Path<String>,
    Json(req): Json<SetLookValueRequest>,
) -> impl IntoResponse {
    dispatch(
        &state,
        EngineCommand::SetLookValue {
            look: uuid,
            target_kind: req.target_kind,
            target: req.target,
            role: req.role,
            value: req.value,
            palette: req.palette,
        },
    )
    .await
}

/// Instantiate a look as a lighting deck in a channel.
#[utoipa::path(post, path = "/api/lighting/decks", request_body = AddLightingDeckRequest,
    responses((status = 200, description = "Deck created; returns its UUID", body = CommandResult),
              (status = 404, description = "No such look")),
    tag = "Lighting")]
pub async fn add_lighting_deck(
    State(state): State<SharedState>,
    Json(req): Json<AddLightingDeckRequest>,
) -> impl IntoResponse {
    dispatch(
        &state,
        EngineCommand::AddLightingDeck {
            channel: req.channel,
            look: req.look,
        },
    )
    .await
}

/// Update a lighting deck, or move it to another channel.
#[utoipa::path(patch, path = "/api/lighting/decks/{uuid}",
    params(("uuid" = String, Path, description = "Lighting deck UUID")),
    request_body = UpdateLightingDeckRequest,
    responses((status = 200, description = "Deck updated", body = CommandResult),
              (status = 404, description = "No such deck")),
    tag = "Lighting")]
pub async fn update_lighting_deck(
    State(state): State<SharedState>,
    Path(uuid): Path<String>,
    Json(req): Json<UpdateLightingDeckRequest>,
) -> impl IntoResponse {
    // A move is a distinct command so the engine can keep the deck's settings intact rather
    // than the caller having to delete and recreate it.
    if let Some(channel) = req.channel {
        return dispatch(&state, EngineCommand::MoveLightingDeck { uuid, channel }).await;
    }
    dispatch(
        &state,
        EngineCommand::UpdateLightingDeck {
            uuid,
            level: req.level,
            blend: req.blend,
            mute: req.mute,
            solo: req.solo,
            independent: req.independent,
            ltp_transition: req.ltp_transition,
        },
    )
    .await
}

/// Remove a lighting deck.
#[utoipa::path(delete, path = "/api/lighting/decks/{uuid}",
    params(("uuid" = String, Path, description = "Lighting deck UUID")),
    responses((status = 200, description = "Deck removed", body = CommandResult),
              (status = 404, description = "No such deck")),
    tag = "Lighting")]
pub async fn remove_lighting_deck(
    State(state): State<SharedState>,
    Path(uuid): Path<String>,
) -> impl IntoResponse {
    dispatch(&state, EngineCommand::RemoveLightingDeck { uuid }).await
}

/// Create a palette. Its kind decides whether it is saved with the show or the venue.
#[utoipa::path(post, path = "/api/lighting/palettes", request_body = AddPaletteRequest,
    responses((status = 200, description = "Palette created; returns its UUID", body = CommandResult),
              (status = 400, description = "Unknown palette kind")),
    tag = "Lighting")]
pub async fn add_palette(
    State(state): State<SharedState>,
    Json(req): Json<AddPaletteRequest>,
) -> impl IntoResponse {
    dispatch(
        &state,
        EngineCommand::AddPalette {
            name: req.name,
            kind: req.kind,
        },
    )
    .await
}

/// Remove a palette. Looks referencing it become inert rather than dark.
#[utoipa::path(delete, path = "/api/lighting/palettes/{uuid}",
    params(("uuid" = String, Path, description = "Palette UUID")),
    responses((status = 200, description = "Palette removed", body = CommandResult),
              (status = 404, description = "No such palette")),
    tag = "Lighting")]
pub async fn remove_palette(
    State(state): State<SharedState>,
    Path(uuid): Path<String>,
) -> impl IntoResponse {
    dispatch(&state, EngineCommand::RemovePalette { uuid }).await
}

/// Store a value into a palette, as the role-space default or as a per-fixture override.
#[utoipa::path(put, path = "/api/lighting/palettes/{uuid}/value",
    params(("uuid" = String, Path, description = "Palette UUID")),
    request_body = SetPaletteValueRequest,
    responses((status = 200, description = "Value stored", body = CommandResult),
              (status = 404, description = "No such palette")),
    tag = "Lighting")]
pub async fn set_palette_value(
    State(state): State<SharedState>,
    Path(uuid): Path<String>,
    Json(req): Json<SetPaletteValueRequest>,
) -> impl IntoResponse {
    dispatch(
        &state,
        EngineCommand::SetPaletteValue {
            palette: uuid,
            role: req.role,
            value: req.value,
            fixture: req.fixture,
        },
    )
    .await
}

/// Set the global lighting intensity scalar.
#[utoipa::path(post, path = "/api/lighting/master", request_body = FloatRequest,
    responses((status = 200, description = "Master set", body = CommandResult)),
    tag = "Lighting")]
pub async fn set_master(
    State(state): State<SharedState>,
    Json(req): Json<FloatRequest>,
) -> impl IntoResponse {
    dispatch(&state, EngineCommand::SetLightingMaster(req.value)).await
}

/// Release every programmer value, handing control back to the looks.
#[utoipa::path(post, path = "/api/lighting/release",
    responses((status = 200, description = "Programmer released", body = CommandResult)),
    tag = "Lighting")]
pub async fn release_programmer(State(state): State<SharedState>) -> impl IntoResponse {
    dispatch(&state, EngineCommand::ReleaseLightingProgrammer).await
}

/// Body of `POST /api/lighting/looks/{uuid}/store`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct StoreToLookRequest {
    /// `group` or `fixture`.
    pub target_kind: String,
    pub target: String,
}

/// Commit what the programmer holds into a look.
///
/// The console workflow is select, set, store. This is the store half: it reads the programmer
/// rather than taking values inline, so a headless client and the window commit exactly the
/// same thing.
#[utoipa::path(post, path = "/api/lighting/looks/{uuid}/store",
    params(("uuid" = String, Path, description = "Look UUID")),
    request_body = StoreToLookRequest,
    responses((status = 200, description = "Stored; returns how many roles were written", body = CommandResult),
              (status = 400, description = "The programmer is empty, or the target is malformed"),
              (status = 404, description = "No such look")),
    tag = "Lighting")]
pub async fn store_to_look(
    State(state): State<SharedState>,
    Path(uuid): Path<String>,
    Json(req): Json<StoreToLookRequest>,
) -> impl IntoResponse {
    dispatch(
        &state,
        EngineCommand::StoreProgrammerToLook {
            look: uuid,
            target_kind: req.target_kind,
            target: req.target,
        },
    )
    .await
}

/// Commit what the programmer holds into a palette.
#[utoipa::path(post, path = "/api/lighting/palettes/{uuid}/store",
    params(("uuid" = String, Path, description = "Palette UUID")),
    responses((status = 200, description = "Stored; position palettes store one entry per fixture", body = CommandResult),
              (status = 400, description = "The programmer is empty"),
              (status = 404, description = "No such palette")),
    tag = "Lighting")]
pub async fn store_to_palette(
    State(state): State<SharedState>,
    Path(uuid): Path<String>,
) -> impl IntoResponse {
    dispatch(
        &state,
        EngineCommand::StoreProgrammerToPalette { palette: uuid },
    )
    .await
}

/// Enable or disable the whole lighting subsystem.
#[utoipa::path(post, path = "/api/lighting/enabled",
    request_body = BoolRequest,
    responses(
        (status = 200, description = "Subsystem toggled", body = CommandResult),
        (status = 400, description = "Enabled, but the patch does not resolve"),
        (status = 503, description = "Engine not yet initialized")),
    tag = "Lighting")]
pub async fn set_enabled(
    State(state): State<SharedState>,
    Json(req): Json<BoolRequest>,
) -> impl IntoResponse {
    dispatch(&state, EngineCommand::SetLightingEnabled(req.value)).await
}
