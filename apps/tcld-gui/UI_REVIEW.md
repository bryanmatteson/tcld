# Desktop UI Notes

This document records the desktop interface structure and the remaining design
work. Source and interaction tests are authoritative.

## Workspace model

The desktop application uses a binary mosaic layout with a tab stack in each
leaf. Layouts, active views, and split percentages persist locally. Tabs can be
reordered, moved between leaves, or dropped at an edge to create a split.

The workspace has three primary state boundaries:

- `useTcldStore` owns loaded artifacts and analysis details.
- `useDebugStore` owns the active debugger session.
- `useLocationStore` owns the shared cursor, history, marks, and pins.

Panels read these stores through typed models. Tauri calls remain in
`tauri-api.ts`; panel components do not implement bytecode or debugger logic.

## Interaction model

Selection is shared across disassembly, reconstructed source, source maps, and
detail views. A location change in one panel updates the others. Hover state is
temporary and travels through `hoverBus`.

The command registry is the common index for keyboard shortcuts, the command
palette, and native menu coverage. Panel-local actions stay in panel menus when
they do not have application-wide meaning.

The command palette filters actions by the current workspace and debugger
state, puts common tasks first, matches multiword queries across titles and
categories, keeps the keyboard selection in view, and returns focus to its launcher.
The palette, examples, and keyboard reference contain keyboard focus and close
with Escape. History and saved-mark popovers accept clicks inside their portal
before dismissing. Back and Forward navigate on a single click; the separate
history picker also supports arrow-key selection.

The welcome screen offers file inspection, bundled examples, and script debugging
when the runtime supports it. The panel chooser searches names and purpose
descriptions, distinguishes open panels from new ones, and focuses existing
panels instead of duplicating them. Layout presets explain their intended task;
saved layouts list human-readable panel names.

File loading leaves the current workspace mounted and shows inline progress.
The most recent open or document selection wins over older pending requests;
closing a document cancels pending activation. A later open reads the file again
so edits on disk are visible. Failed opens retain the current document and show
a dismissible error. Bundled examples load through `load_example`, which embeds
real source/bytecode pairs. `load_text` uses the shared source compiler instead
of misclassifying plain source as a debugger payload.

The debugger exposes only capabilities reported by the backend. Unsupported
actions remain visible when context is useful, with a short explanation of the
missing capability.

## Visual system

Theme tokens define color, type, spacing, borders, shadows, and overlay
surfaces. Shared primitives cover chips, fields, collapsible sections, tab
badges, empty states, and panel chrome. New components should use these tokens
before introducing local values.

The interface distinguishes:

- the focused panel;
- the selected analysis location;
- the active debugger program counter;
- temporary hover relationships;
- persistent marks, pins, and breakpoints.

These states should remain visually distinct in every theme and density.

Chrome text — tabs, menus, buttons, labels, and messages — uses the UI face
(`--ui`, Geist in the default Modern Sans type system). Code, addresses,
operands, literals, and keyboard hints use the mono face. Mono Brutalist stays
available in Preferences for an all-mono interface. Size chrome text with the
semantic utilities (`text-ui`, `text-body`, `text-control`, `text-caption`,
`text-meta`, `text-label`, `text-tag`), never arbitrary pixel sizes.

The accent marks focus and the primary action. Only the focused panel's active
tab carries the accent underline; other active tabs use a neutral one.
Debugging uses the same type scale and control sizes as inspection, so starting
a session never reflows the workspace.

The Disassembly shows what an operand means: a push shows its literal (strings
quoted, numbers bare), a jump shows its target address and direction, and an
invoke shows its word count. The raw encoding stays in the row tooltip.
Addresses use one spelling (`0064`) in the address column, jump operands, and
jump-target markers. Opcode color follows role: stack plumbing is muted,
variable access blue, calls purple, and every control-flow opcode amber; red
stays reserved for errors and breakpoints. In Grouped mode a pinned header
names the command the top row belongs to.

Panel toolbars use the shared primitives: a `tcld-toolbar-group` with
`role="tablist"` for mode switches (rendered as a segmented control),
`tcld-toolbar-icon` for icon actions, `tcld-toolbar-pill` for text actions, and
`tcld-toolbar-meta` for counts. Every icon-only control needs a tooltip and an
accessible name. Labels and status messages use plain language and the panel
names users see, not internal view ids.

## Data and rendering

Artifact loading produces one domain snapshot. Panels derive their models from
that snapshot rather than requesting independent interpretations. Expensive
instruction, graph, and location indexes should be memoized by snapshot
identity.

Debugger events are authoritative for running state. Explicit refresh calls are
reserved for recovery and user-requested synchronization.

Large listings use virtualized rows. Asynchronous detail requests should be
deduplicated and ignored when a newer cursor request supersedes them.

A cursor move should re-render only what shows the cursor. Panels read the
artifact with `useTcldData` (not `useTcldStore`, which also changes with
per-document session state) and read `useLocationStore` through selectors
(`useShallow`), never the whole store. Memoized rows take primitive props and
stable callbacks that receive the row's own id; derived models (folds, filters,
group metadata) must not depend on the cursor unless they display it. Effects
that cross the IPC boundary send only when their payload changes. Fixed-height
lists reveal the cursor with `revealRow`, which keeps a few rows of context and
skips scrolling when the row is already in view.

## Remaining work

The following changes would simplify the interface without changing its
product model:

1. Move panel metadata, rendering, badges, and commands into one typed view
   registry.
2. Complete keyboard navigation between leaves and tabs.
3. Label split targets while dragging.
4. Keep all panel bodies on the shared empty, loading, error, field, and
   collapsible primitives.
5. Memoize derived indexes and deduplicate concurrent detail requests.
6. Keep native menus, the command palette, and help text generated from the
   same action registry.

## Verification

```bash
npm test
npm run design:parity
npm run build
```

The test suite covers layout persistence, action coverage, location
synchronization, debugger capability degradation, keyboard behavior, and
production Tauri integration.
