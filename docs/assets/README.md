# Tcld native workbench media

`icon.svg` is an editable vector mark for bytecode and source investigation.
`workbench.gif` and the JPEGs are captures of the native embedded Tauri app
opening `testing/fixtures/public/public_complex_fixture.tbc`. They show
instructions, linked source navigation, symbols, and source comparison.
No browser fixture or frontend mock is used.

## Prepare

On macOS, with the project's Tcl and frontend prerequisites available:

```sh
python3 scripts/readme/prepare.py
```

The script builds the frontend and release host with `tauri/custom-protocol`,
then copies the real host into
`~/Documents/Developer/tcld/readme/TCLD.app`. Its separate bundle identifier
keeps recording preferences separate from the normal app. Use `--skip-build`
only to reuse an already current executable and frontend.

## Capture

Open the recording app, open the public fixture, and select **Default** from
Layout. Keep the default 1400 × 900 window, with Source View visible. In a
Codex CUA session:

```js
const app = await cua.getApp('dev.matteson.tcld.readme');
const {record} = await import('/absolute/path/to/tcld/scripts/readme/record.mjs');
await record(app);
```

The [recorder](../../scripts/readme/record.mjs) checks the current accessibility
state before selecting instructions or symbols. It uses screenshot-relative
coordinates for tab headers that have no separate accessibility control, and
checks that source navigation reaches the selected PC. Screenshots and
`frames.json` go to `~/Documents/Developer/tcld/readme/frames`.

With Python 3 and FFmpeg installed:

```sh
python3 scripts/readme/encode.py ~/Documents/Developer/tcld/readme/frames
```

The [encoder](../../scripts/readme/encode.py) assembles a looping, 20-second GIF
from the actual interaction states and publishes the stills here. Captures do
not include the desktop or file chooser. The older terminal demo remains in
[`docs/demo.tape`](../demo.tape).
