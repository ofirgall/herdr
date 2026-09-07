# Middle Mouse Forwarding Specification

## Purpose

Ensures middle mouse down events reach the terminal pane so applications with mouse reporting enabled can respond to middle-click.

## Requirements

### Requirement: Middle mouse down forwarded to terminal pane

When the user middle-clicks on a terminal pane in the main (non-popup) view, the down event SHALL be forwarded to the pane via the same `forward_pane_mouse_button` path used for middle mouse up and drag.

#### Scenario: Application with mouse reporting receives middle-click
- **WHEN** a terminal application has mouse reporting enabled
- **AND** the user middle-clicks on that pane (not in the sidebar)
- **THEN** the pane SHALL receive the middle mouse down event

#### Scenario: Middle-click focuses the pane
- **WHEN** the user middle-clicks on a non-focused pane
- **THEN** the pane SHALL be focused (existing behavior preserved)
- **AND** the middle mouse down event SHALL be forwarded to the pane

#### Scenario: Middle-click on sidebar is ignored
- **WHEN** the user middle-clicks in the sidebar area
- **THEN** the event SHALL NOT be forwarded to any terminal pane

#### Scenario: Application without mouse reporting
- **WHEN** a terminal application has NOT enabled mouse reporting
- **AND** the user middle-clicks on that pane
- **THEN** `forward_pane_mouse_button` returns false and no bytes are sent (existing ghostty behavior)
