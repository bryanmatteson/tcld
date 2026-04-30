// Removed in the UI cleanup pass — the global breadcrumb row was replaced by
// the slimmer NavHistoryControls component (back/forward + history popover)
// docked into the FileHeader titlebar trail. Scope info that this row used
// to render is already covered by panel badges and the StatusBar position
// counter, so the duplicate row was removed entirely.
//
// This file is kept as an empty module so existing import paths fail loudly
// at type-check time rather than silently resolving to a stale component.
// Delete the file in a follow-up commit when no references remain.
export {};
