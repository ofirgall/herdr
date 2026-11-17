# Spec Delta

## Purpose

Defines how spaces get jump numbers, which keys reach them (including a tenth space on `0`), how a user skips a space from numbering, and where the numbers are shown.

## ADDED Requirements

### Requirement: Jump numbers follow sidebar order and skip flagged spaces
The client SHALL assign jump numbers to spaces in the order `switch_workspace` uses today (sidebar navigation order), skipping spaces whose skip-index flag is set. The first nine numbered spaces SHALL get `1`–`9` and the tenth SHALL get `0`. Later spaces and skipped spaces SHALL have no number.

#### Scenario: No skipped spaces
- **WHEN** no space is skipped
- **THEN** the space in sidebar position N (N ≤ 9) SHALL have number N, as today

#### Scenario: Skipped space
- **WHEN** the spaces in order are A, B, C and B is skipped
- **THEN** A SHALL have number 1, B SHALL have none and C SHALL have number 2

#### Scenario: Tenth space
- **WHEN** there are eleven spaces and none is skipped
- **THEN** the tenth SHALL have number `0` and the eleventh SHALL have none

### Requirement: Indexed space keys reach numbered spaces
A `switch_workspace` binding for number N SHALL focus the space with jump number N, and do nothing when no space has that number. Previous/next space keys, clicks and the navigator list SHALL still reach skipped spaces.

#### Scenario: Key skips over a skipped space
- **WHEN** A, B, C are in order, B is skipped and the user presses the key for 2
- **THEN** C SHALL be focused

#### Scenario: Next space visits a skipped space
- **WHEN** A is focused, B is skipped and the user presses next space
- **THEN** B SHALL be focused

### Requirement: The 0..9 range binds the tenth space
`switch_workspace` SHALL accept the range `0..9` (e.g. `ctrl+0..9`), which binds keys 1–9 to numbers 1–9 and key 0 to number `0`. It SHALL also accept a single binding on key `0`. `1..9` SHALL keep binding only 1–9. `switch_tab` and `focus_agent` SHALL reject `0..9` and key `0` with the existing indexed-binding diagnostic.

#### Scenario: Range with zero
- **WHEN** `switch_workspace = "ctrl+0..9"` and the user presses `ctrl+0` with eleven spaces
- **THEN** the tenth space SHALL be focused

#### Scenario: Tabs keep 1..9
- **WHEN** `switch_tab = "prefix+0..9"`
- **THEN** config check SHALL report the indexed-binding diagnostic and the binding SHALL be disabled

### Requirement: Navigate mode uses jump numbers
In navigate mode, digit keys SHALL select the space with that jump number, and `0` SHALL select the space numbered `0`.

#### Scenario: Navigate zero
- **WHEN** the user is in navigate mode with eleven spaces and presses `0`
- **THEN** the tenth space SHALL be selected

### Requirement: Toggle the focused space's skip flag
A new keybinding `toggle_space_index`, unbound by default, SHALL flip the focused space's skip-index flag through the API. The space context menu SHALL offer "Skip jump number" or "Use jump number" according to the space's current flag. When the server does not support the method, the action SHALL show the existing unsupported-method notice and change nothing.

#### Scenario: Toggle by key
- **WHEN** `toggle_space_index = "prefix+i"` and the user presses it on space B
- **THEN** B SHALL become skipped and the numbers after it SHALL shift down

#### Scenario: Context menu
- **WHEN** the user opens the context menu of a skipped space
- **THEN** it SHALL offer "Use jump number"

### Requirement: The index token shows the jump number
Space rows SHALL accept a new `index` token that renders the space's jump number (`1`–`9` or `0`). It SHALL resolve to no value, and be elided like any missing token, for a skipped or unnumbered space. It SHALL support the usual token styles and rules. It SHALL NOT be added to the default rows.

#### Scenario: Numbered rows
- **WHEN** `rows = [["index", "workspace"]]` and B of A, B, C is skipped
- **THEN** the rows SHALL read `1 A`, `B`, `2 C`

### Requirement: The collapsed sidebar shows jump numbers
The collapsed sidebar SHALL show each space's jump number instead of its list position, and nothing for skipped or unnumbered spaces.

#### Scenario: Collapsed with skipped space
- **WHEN** the sidebar is collapsed and B of A, B, C is skipped
- **THEN** A SHALL show `1`, B SHALL show no number and C SHALL show `2`
