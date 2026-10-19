//! Optional endpoint extension that carries SGR 58 underline colors.
//!
//! The frozen v1 cell codec has no underline color, so a negotiated peer receives
//! its canonical surface message (full, reuse, delta, or scroll encoded) wrapped
//! with a sparse color map keyed by surface position. Messages whose cells carry no
//! underline color are never wrapped.

use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};

use super::{FrameData, PaneSurfaceFrame, PaneSurfacePatch, ServerMessage};

pub(crate) const CAPABILITY: &str = "surface_underline_color";
pub(crate) const MESSAGE_KIND: &str = "endpoint.surface-underline.v1";

/// Underline colors as `(x, y, packed color)`, sorted by `(y, x)`.
type PositionedColors = Vec<(u16, u16, u32)>;

/// Underline colors of one surface update.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct UnderlineColors {
    cells: PositionedColors,
    #[serde(default)]
    popup: PositionedColors,
}

impl UnderlineColors {
    /// Every underline color of a complete surface, or `None` when it has none.
    pub(crate) fn of_surface(surface: &PaneSurfaceFrame) -> Option<Box<Self>> {
        let cells = frame_colors(&surface.frame);
        let popup = surface
            .popup
            .as_ref()
            .map(|popup| frame_colors(&popup.frame))
            .unwrap_or_default();
        (!cells.is_empty() || !popup.is_empty()).then(|| Box::new(Self { cells, popup }))
    }

    /// Every underline color of `last` after `patch` applies, or `None` when it has
    /// none. The whole surface is listed because a scroll-encoded patch expands on
    /// the peer into rows beyond the patch's own changed cells.
    pub(crate) fn of_patch(last: &PaneSurfaceFrame, patch: &PaneSurfacePatch) -> Option<Box<Self>> {
        let frame = &last.frame;
        let has_color =
            |cells: &[super::CellData]| cells.iter().any(|cell| cell.underline_color.is_some());
        if !has_color(&frame.cells) && !patch.rows.iter().any(|row| has_color(&row.cells)) {
            return None;
        }
        let mut colors = frame
            .cells
            .iter()
            .map(|cell| cell.underline_color)
            .collect::<Vec<_>>();
        for row in &patch.rows {
            let start = usize::from(row.y) * usize::from(frame.width) + usize::from(row.x);
            if let Some(span) = colors.get_mut(start..start + row.cells.len()) {
                for (color, cell) in span.iter_mut().zip(&row.cells) {
                    *color = cell.underline_color;
                }
            }
        }
        let width = usize::from(frame.width.max(1));
        let cells = colors
            .into_iter()
            .enumerate()
            .filter_map(|(index, color)| {
                Some((
                    u16::try_from(index % width).ok()?,
                    u16::try_from(index / width).ok()?,
                    color?,
                ))
            })
            .collect::<Vec<_>>();
        (!cells.is_empty()).then(|| {
            Box::new(Self {
                cells,
                popup: Vec::new(),
            })
        })
    }
}

fn frame_colors(frame: &FrameData) -> PositionedColors {
    let width = usize::from(frame.width.max(1));
    frame
        .cells
        .iter()
        .enumerate()
        .filter_map(|(index, cell)| {
            let color = cell.underline_color?;
            let x = u16::try_from(index % width).ok()?;
            let y = u16::try_from(index / width).ok()?;
            Some((x, y, color))
        })
        .collect()
}

#[derive(Serialize, Deserialize)]
struct SurfaceUnderline {
    /// One framed canonical `ServerMessage`, base64 encoded.
    message: String,
    colors: UnderlineColors,
}

/// Wraps an already encoded surface message with its underline colors. Returns `None`
/// when the wrapper cannot be encoded within the frame limit; the caller then sends
/// the canonical message and the peer shows plain underlines for that update.
pub(crate) fn wrap(message: &ServerMessage, colors: &UnderlineColors) -> Option<ServerMessage> {
    let mut framed = Vec::new();
    if let Err(error) = super::write_message(&mut framed, message) {
        tracing::warn!(%error, "failed to encode underline-wrapped surface");
        return None;
    }
    let data = serde_json::to_string(&SurfaceUnderline {
        message: STANDARD_NO_PAD.encode(framed),
        colors: colors.clone(),
    })
    .map_err(|error| tracing::warn!(%error, "failed to encode surface underline"))
    .ok()?;
    let wrapped = ServerMessage::EndpointControl {
        kind: MESSAGE_KIND.into(),
        data,
    };
    let size = bincode::serde::encode_into_std_write(
        &wrapped,
        &mut std::io::sink(),
        bincode::config::standard(),
    )
    .ok()?;
    (size <= super::MAX_FRAME_SIZE).then_some(wrapped)
}

/// Splits a wrapper into its canonical message and colors.
pub(crate) fn unwrap(data: &str) -> Result<(ServerMessage, UnderlineColors), String> {
    let wrapper: SurfaceUnderline = serde_json::from_str(data)
        .map_err(|error| format!("invalid surface underline: {error}"))?;
    let framed = STANDARD_NO_PAD
        .decode(wrapper.message)
        .map_err(|error| format!("invalid surface underline: {error}"))?;
    let message: ServerMessage = super::read_message(&mut framed.as_slice(), super::MAX_FRAME_SIZE)
        .map_err(|error| format!("invalid surface underline: {error}"))?;
    if matches!(&message, ServerMessage::EndpointControl { kind, .. } if kind == MESSAGE_KIND) {
        return Err("surface underline cannot be nested".into());
    }
    Ok((message, wrapper.colors))
}

/// Applies the colors of a complete surface. They are authoritative, so cells
/// retained from an earlier baseline lose colors the server no longer reports.
pub(crate) fn apply_to_surface(
    surface: &mut PaneSurfaceFrame,
    colors: &UnderlineColors,
) -> Result<(), String> {
    apply_to_frame(&mut surface.frame, &colors.cells)?;
    match &mut surface.popup {
        Some(popup) => apply_to_frame(&mut popup.frame, &colors.popup),
        None if colors.popup.is_empty() => Ok(()),
        None => Err("surface underline has popup colors without a popup".into()),
    }
}

fn apply_to_frame(frame: &mut FrameData, colors: &[(u16, u16, u32)]) -> Result<(), String> {
    for cell in &mut frame.cells {
        cell.underline_color = None;
    }
    for &(x, y, color) in colors {
        let cell = (x < frame.width)
            .then(|| usize::from(y) * usize::from(frame.width) + usize::from(x))
            .and_then(|index| frame.cells.get_mut(index))
            .ok_or("surface underline exceeds its surface")?;
        cell.underline_color = Some(color);
    }
    Ok(())
}

/// Applies a patch's post-update color map to every cell the decoded patch carries.
pub(crate) fn apply_to_patch(
    patch: &mut PaneSurfacePatch,
    colors: &UnderlineColors,
) -> Result<(), String> {
    if !colors.popup.is_empty() {
        return Err("surface underline has popup colors for a patch".into());
    }
    if !colors
        .cells
        .windows(2)
        .all(|pair| (pair[0].1, pair[0].0) < (pair[1].1, pair[1].0))
    {
        return Err("surface underline patch colors are not ordered".into());
    }
    for row in &mut patch.rows {
        for (offset, cell) in row.cells.iter_mut().enumerate() {
            let position = (row.y, usize::from(row.x) + offset);
            cell.underline_color = colors
                .cells
                .binary_search_by_key(&position, |&(x, y, _)| (y, usize::from(x)))
                .ok()
                .map(|index| colors.cells[index].2);
        }
    }
    Ok(())
}
