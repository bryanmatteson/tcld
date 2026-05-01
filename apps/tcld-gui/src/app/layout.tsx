import '@/tauriConsole';
import type { Metadata } from "next";
import "./globals.css";
import { TcldStoreProvider } from "@/lib/store";
import { DebugStoreProvider } from "@/lib/debug-store";
import { LocationStoreProvider } from "@/lib/location-store";

export const metadata: Metadata = {
  title: "Tcld",
  description: "Tcl bytecode analysis and debugging",
};

// Inline script to apply saved settings before first paint (prevents theme flash).
// Must be a plain string to run as a blocking <script> in <head>.
const SETTINGS_INIT_SCRIPT = `
(function(){
  try {
    var s = JSON.parse(localStorage.getItem('tcld-settings') || '{}');
    var r = document.documentElement.style;
    if (s.theme) {
      var t = window.__TCLD_THEMES && window.__TCLD_THEMES[s.theme];
      if (t) for (var k in t) r.setProperty(k, t[k]);
    }
    if (s.fontFamily) {
      var f = window.__TCLD_FONTS && window.__TCLD_FONTS[s.fontFamily];
      if (f) r.setProperty('--font-mono', f);
    }
    if (s.fontSize) r.setProperty('--font-size-base', s.fontSize + 'px');
    var density = s.density || 'regular';
    document.documentElement.dataset.density = density;
    if (density === 'compact') {
      r.setProperty('--code-row-height', '20px');
      r.setProperty('--panel-header-height', '24px');
      r.setProperty('--toolbar-mode', 'icons');
    } else if (density === 'comfy') {
      r.setProperty('--code-row-height', '30px');
      r.setProperty('--panel-header-height', '39px');
      r.setProperty('--toolbar-mode', 'full');
    } else {
      r.setProperty('--code-row-height', '24px');
      r.setProperty('--panel-header-height', '30px');
      r.setProperty('--toolbar-mode', 'full');
    }
  } catch(e) {}
})();
`;

// Embed theme/font lookup tables so the init script can resolve them synchronously.
import { THEMES, FONTS } from "@/lib/themes";
const THEME_COLORS_JSON = JSON.stringify(
  Object.fromEntries(Object.entries(THEMES).map(([id, def]) => [id, def.colors]))
);
const FONT_STACKS_JSON = JSON.stringify(
  Object.fromEntries(Object.entries(FONTS).map(([id, def]) => [id, def.stack]))
);

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning>
      <head>
        <link rel="preconnect" href="https://fonts.googleapis.com" />
        <link rel="preconnect" href="https://fonts.gstatic.com" crossOrigin="anonymous" />
        {/* eslint-disable-next-line @next/next/no-page-custom-font --
            False positive: the rule targets per-page fonts in the pages/
            router; this is the app-router root layout, which wraps every
            route, so the stylesheet loads exactly once for the whole app. */}
        <link
          rel="stylesheet"
          href="https://fonts.googleapis.com/css2?family=Geist+Mono:wght@400;500;600;700&family=Geist:wght@400;500;600;700&display=swap"
        />
        <script
          dangerouslySetInnerHTML={{
            __html: `window.__TCLD_THEMES=${THEME_COLORS_JSON};window.__TCLD_FONTS=${FONT_STACKS_JSON};${SETTINGS_INIT_SCRIPT}`,
          }}
        />
      </head>
      <body className="m-0 p-0 overflow-hidden bg-tcld-bg text-tcld-fg font-mono antialiased">
        <TcldStoreProvider>
          <DebugStoreProvider>
            <LocationStoreProvider>{children}</LocationStoreProvider>
          </DebugStoreProvider>
        </TcldStoreProvider>
      </body>
    </html>
  );
}
