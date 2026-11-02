//! Optional client-shell report of the full host window size. Servers advertising the
//! capability size the session popup against this window instead of the pane surface,
//! so client chrome such as the sidebar does not move or resize the popup.

use super::{ClientMessage, ClientSurfaceSize};
use serde::{Deserialize, Serialize};

pub(crate) const CAPABILITY: &str = "shell_window_size";
pub(crate) const MESSAGE_KIND: &str = "endpoint.shell-window-size.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ShellWindowSize {
    pub(crate) cols: u16,
    pub(crate) rows: u16,
}

impl From<ShellWindowSize> for ClientSurfaceSize {
    fn from(size: ShellWindowSize) -> Self {
        Self {
            cols: size.cols,
            rows: size.rows,
        }
    }
}

pub(crate) fn message(size: ShellWindowSize) -> serde_json::Result<ClientMessage> {
    Ok(ClientMessage::EndpointControl {
        kind: MESSAGE_KIND.into(),
        data: serde_json::to_string(&size)?,
    })
}

pub(crate) fn decode(data: &str) -> Result<ShellWindowSize, String> {
    serde_json::from_str(data).map_err(|error| format!("invalid shell window size: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_round_trips_through_endpoint_control() {
        let size = ShellWindowSize {
            cols: 160,
            rows: 48,
        };
        let ClientMessage::EndpointControl { kind, data } = message(size).unwrap() else {
            panic!("expected endpoint control");
        };
        assert_eq!(kind, MESSAGE_KIND);
        assert_eq!(decode(&data).unwrap(), size);
        assert!(decode("{\"cols\":-1,\"rows\":2}").is_err());
    }
}
