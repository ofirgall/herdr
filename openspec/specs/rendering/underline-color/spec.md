# Underline Color Specification

## Purpose

Ensures underline color (SGR 58) set by programs in herdr panes is preserved through the wire protocol and rendered to the outer terminal, so colored undercurls display correctly.

## Requirements

### Requirement: Underline color preserved in wire protocol

The wire protocol's cell representation SHALL carry the underline color alongside fg, bg, and modifier. When a cell has an underline color set (via SGR 58), it MUST be encoded and transmitted to the rendering client.

#### Scenario: Cell with RGB underline color
- **WHEN** a program in a pane emits SGR 58 with an RGB color (e.g., `\e[58:2::255:0:0m`)
- **THEN** the cell's underline color SHALL be preserved in the wire protocol cell data with the full RGB value

#### Scenario: Cell with no underline color
- **WHEN** a program in a pane does not set an underline color
- **THEN** the cell's underline color SHALL be absent (None/default), and no SGR 58 SHALL be emitted in the ANSI output

### Requirement: Underline color emitted in ANSI rendering

The ANSI renderer SHALL emit SGR 58 in the escape sequence when a cell has an underline color set. The emitted sequence MUST match the color type (RGB, indexed, or named).

#### Scenario: RGB underline color rendered
- **WHEN** a cell has an RGB underline color (r, g, b)
- **THEN** the renderer SHALL emit `58:2::R:G:B` as part of the SGR sequence

#### Scenario: Indexed underline color rendered
- **WHEN** a cell has an indexed (256-color) underline color
- **THEN** the renderer SHALL emit `58:5:INDEX` as part of the SGR sequence

#### Scenario: Underline color reset on style change
- **WHEN** the current cell has no underline color but the previous cell did
- **THEN** the renderer SHALL emit SGR 59 (underline color reset) or a full SGR reset

### Requirement: Underline color participates in cell diff

Cell comparison logic SHALL include the underline color when determining whether two cells are visually identical, so that underline color changes trigger re-rendering.

#### Scenario: Differing underline color triggers update
- **WHEN** two cells have identical symbol, fg, bg, and modifier but different underline colors
- **THEN** the cells SHALL be considered visually different and the new cell SHALL be re-rendered
