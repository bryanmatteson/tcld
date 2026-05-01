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

## Data and rendering

Artifact loading produces one domain snapshot. Panels derive their models from
that snapshot rather than requesting independent interpretations. Expensive
instruction, graph, and location indexes should be memoized by snapshot
identity.

Debugger events are authoritative for running state. Explicit refresh calls are
reserved for recovery and user-requested synchronization.

Large listings use virtualized rows. Asynchronous detail requests should be
deduplicated and ignored when a newer cursor request supersedes them.

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
