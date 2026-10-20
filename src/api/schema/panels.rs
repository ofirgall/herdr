//! Server-owned terminal panels: `panel.list`, `panel.restart` and the two
//! per-connection endpoint methods `panel.surface.set` and `panel.input`.

use serde::{Deserialize, Serialize};

use crate::cli::projection::{cli_params, Field, Input};

pub(crate) fn panel_id() -> Input<String> {
    Input::positional("id", 0, |slot, value| {
        *slot = value.to_owned();
        Ok(())
    })
    .value_name("PANEL_ID")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PanelRunState {
    Running,
    Exited,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PanelInfo {
    pub id: String,
    pub status: PanelRunState,
    /// Exit code of an exited panel, when the platform reported one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

cli_params! {
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
    pub struct PanelRestartParams => PanelRestartProjection {
        pub id: String => Field::new(String::new()).input(panel_id()),
    }
}

/// Declares one panel's size and whether this connection currently shows it.
/// The panel terminal follows the most recent visible declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PanelSurfaceSetParams {
    pub id: String,
    pub cols: u16,
    pub rows: u16,
    pub visible: bool,
}

/// Semantic input for a panel terminal. Each event is a serialized
/// `ClientPaneInputEvent`, the same shape `shell.input.semantic.v1` carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PanelInputParams {
    pub id: String,
    pub events: Vec<serde_json::Value>,
}
