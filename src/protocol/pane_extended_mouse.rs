//! Optional client-shell pane input for mouse buttons the frozen endpoint mouse
//! enum cannot carry. A client sends it only to servers advertising the capability.

use super::ClientMessage;
use crate::input::mouse::ExtendedMouseButton;
use serde::{Deserialize, Serialize};

pub(crate) const CAPABILITY: &str = "pane_extended_mouse";
pub(crate) const MESSAGE_KIND: &str = "endpoint.pane-extended-mouse.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PaneExtendedMouse {
    pub(crate) pane_id: String,
    pub(crate) button: ExtendedMouseButton,
    pub(crate) pressed: bool,
    /// 0-based cell within the pane.
    pub(crate) column: u16,
    pub(crate) row: u16,
    /// `crossterm::event::KeyModifiers` bits.
    pub(crate) modifiers: u8,
}

pub(crate) fn message(input: &PaneExtendedMouse) -> serde_json::Result<ClientMessage> {
    Ok(ClientMessage::EndpointControl {
        kind: MESSAGE_KIND.into(),
        data: serde_json::to_string(input)?,
    })
}

pub(crate) fn decode(data: &str) -> Result<PaneExtendedMouse, String> {
    serde_json::from_str(data).map_err(|error| format!("invalid pane extended mouse: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_round_trips_through_endpoint_control() {
        let input = PaneExtendedMouse {
            pane_id: "w1-p2".into(),
            button: ExtendedMouseButton::Forward,
            pressed: true,
            column: 4,
            row: 7,
            modifiers: 0b10,
        };
        let ClientMessage::EndpointControl { kind, data } = message(&input).unwrap() else {
            panic!("expected endpoint control");
        };
        assert_eq!(kind, MESSAGE_KIND);
        assert!(data.contains("\"button\":\"forward\""), "{data}");
        assert_eq!(decode(&data).unwrap(), input);
        assert!(decode("{\"button\":\"sideways\"}").is_err());
    }
}
