

if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
  void import('@tauri-apps/plugin-log')
    .then(({ attachConsole }) => attachConsole())
    .catch((err) => {
      console.warn('[tauri-log] Failed to attach console bridge', err);
    });
}

export { };
