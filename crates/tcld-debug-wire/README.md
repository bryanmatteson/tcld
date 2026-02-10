# tcld-debug-wire

This crate defines Tcld's out-of-process debugger protocol and synchronous Rust
controller. It has no Tcl dependency.

A debugger frontend controls a separate `tclsh` process running the debugger
nub over loopback TCP. The Rust controller and the Tcl controller in
`crates/tcld-dbg-ext/scripts/tcld_ctrl.tcl` use the types and framing described
here.

Protocol version: `PROTOCOL_VERSION = 2` (`src/handshake.rs`). v2
rejects peers that report another version.

## Frame envelope and framing

Every message, in both directions, is one length-prefixed Tcl-list
frame on a single TCP stream (v2: loopback TCP only):

```
<decimal-byte-length>\n<tcl-list-body>\n
```

- The length counts the bytes of `<tcl-list-body>` only (not the
  trailing newline).
- The length prefix is ASCII decimal, at most 10 digits
  (`codec::MAX_LENGTH_DIGITS`).
- The body is capped at 32 MiB (`codec::MAX_BODY_BYTES`); both encoder
  and decoder refuse larger frames.
- The length prefix is required because Tcl lists may contain literal
  newlines inside braced elements — line-oriented framing alone cannot
  delimit frames.

The body is a three-element Tcl list (`src/frame.rs`):

```
<kind> <serial> {<payload>}
```

| Field | Meaning |
|---|---|
| `kind` | one of `req`, `resp`, `evt`, `err`, `hello`, `hello_ack`, `bye` |
| `serial` | `u64`; `req`/`resp`/`err` share the serial that matches request to response; `evt`, `hello`, `hello_ack`, `bye` are sent with serial `0` |
| `payload` | one Tcl-list element (brace-quoted as needed) whose inner shape depends on `kind` |

The decoder requires exactly three elements. Extra elements produce
`FrameParseError::WrongArity`. Elements are
brace-quoted by the emitter when they contain whitespace, braces,
backslashes, `"`, `$`, `[`, `]`, `;`, or are empty (`src/tcl_list.rs`);
both controllers emit the same quoting so their wire bytes are
identical for the same op sequence (verified by the Tier 4 parity
suite).

Frame kinds by direction:

| Kind | Direction | Payload |
|---|---|---|
| `hello` | nub → controller | handshake fields (below) |
| `hello_ack` | controller → nub | handshake fields (below) |
| `req` | controller → nub | `<op> <args…>` |
| `resp` | nub → controller | op-specific result (opaque Tcl list) |
| `err` | nub → controller | `code <enum> message <text>` |
| `evt` | nub → controller | `<event-kind> <k> <v> …` |
| `bye` | either direction | empty |

## Op vocabulary (closed for v2)

The ops are defined by the `Op` enum in `src/op.rs`. A `req` frame's
payload is the op name followed by positional args. Optional args
(`?…?`) are omitted. Integer args: `level` is signed (`i64`);
`offset`, `len`, `count`, `maxlen` are unsigned (`u64`).

Responses are **opaque** at the wire layer: `OpResponse` carries the
raw `resp` payload string, which is whatever the nub's engine command
returned. The nub dispatches each op to a `::tcld::dbg::*` command
(dispatch table in `crates/tcld-dbg-ext/src/nub.rs`); a few op names
differ from their engine command name (noted below).

| Wire op | Wire args | Nub dispatches to `::tcld::dbg::…` |
|---|---|---|
| `start_target` | `path stop_on_entry` | nub target scheduler |
| `step_into` | — | `step_into` |
| `step_over` | — | `step_over` |
| `step_out` | — | `step_out` |
| `continue` | — | `continue` |
| `interrupt` | — | `request_pause` |
| `current_pc` | — | `current_pc` |
| `app_state` | — | `app_state` |
| `inspect_stack` | — | `inspect_stack` |
| `eval_in_frame` | `level script` | `eval_in_frame` |
| `get_level` | — | `get_level` |
| `get_result` | `?maxlen?` | `get_result` |
| `get_variables` | `level` | `get_variables` |
| `get_var` | `level name ?maxlen?` | `get_var` |
| `set_var` | `level name value` | `set_var` |
| `bp_add` | `source qname offset len ?condition?`, `file_line path line ?condition?`, or `instruction pc ?condition?` | breakpoint adapter |
| `bp_remove` | `bp_id` | `bp_remove` |
| `bp_list` | — | `breakpoint_list` |
| `bp_disable` | `bp_id` | `bp_disable` |
| `bp_enable` | `bp_id` | `bp_enable` |
| `bp_move` | `bp_id qname offset len` | `bp_move` |
| `bp_validate` | `qname` | `bp_validate` |
| `bp_set_ignore` | `bp_id count` | `bp_set_ignore` |
| `bp_export` | — | `bp_export` |
| `bp_import` | `serialized` | `bp_import` |
| `wp_add` | `name ops` | `watchpoint_add` |
| `var_breakpoints` | `?name?` | `var_breakpoints` |
| `line_breakpoints` | `?qname? ?offset?` | `line_breakpoints` |
| `proc_list` | — | `list_procs` |
| `proc_body` | `qname` | `proc_body` |
| `proc_location` | `qname` | `proc_location` |
| `block_disable` | `qname` | `block_disable` |
| `block_enable` | `qname` | `block_enable` |
| `coverage` | — | `coverage` |
| `coverage_enable` | `on\|off` | `coverage_enable` |
| `watch_add` | `name expr` | `watch_add` |
| `watch_remove` | `w_id` | `watch_remove` |
| `watch_list` | — | `watch_list` |
| `watch_evaluate` | `level` | `watch_evaluate` |
| `pre_instrument_proc` | `qname body` | `pre_instrument_proc` |
| `pre_instrument_file` | `path body` | `pre_instrument_file` |
| `slave_list` | — | `slave_list` |
| `slave_install` | `name` | `slave_install` |
| `ignore_error` | — | `ignore_error` |
| `quit` | — | `quit` |

Notes:

- `coverage_enable` accepts `on`, `1`, `true`, or `yes` as true;
  anything else is false. The canonical emission is `on` / `off`.
- `line_breakpoints` only accepts `offset` when `qname` is present.
- An op name outside this list fails to parse
  (`OpParseError::UnknownOp`); the nub replies with an `err` frame for
  any op it does not recognize.

## Event vocabulary (nub → controller, fire-and-forget)

Ten kinds, defined as the `WireEvent` enum in `src/event.rs`. An
`evt` frame's payload is the event kind followed by a **flat dict**
(key/value pairs). Parsers ignore unknown keys, so compatible v2 additions do
not break peers. Optional fields are omitted rather than sent as empty
placeholders.

| Event kind | Fields (r = required, o = optional) |
|---|---|
| `target_started` | `path` (r), `status` (r) |
| `target_completed` | `path` (r), `status` (r), `result` (r) |
| `target_load_failed` | `path`, `status`, `message`, `error_code`, `error_info` (all r) |
| `paused` | `location` (r), `frame_count` (r, u32), `reason` (r), `breakpoint` (o), `pc` (o) |
| `resumed` | — |
| `error_paused` | `location` (o), `message` (r), `error_code` (o), `breakpoint` (o) |
| `condition_error` | `breakpoint` (r), `message` (r) |
| `blocks_revalidated` | `block` (r) |
| `nub_started` | `pid` (r, u32), `version` (r), `tcl_version` (r) |
| `nub_stopped` | `reason` (r) |

Field value shapes (as produced by `crates/tcld-dbg-ext/src/nub.rs`):

- `location` is itself a Tcl list with `block_id`, `block_kind`, `line`,
  `offset_start`, and `offset_end`, plus optional `source_path` and
  `qualified_proc` provenance.
- `<block-id>` is the `BlockId::as_string` form: `proc:<qname>`,
  `file:<path>`, or `anon:<HEX>`
- `breakpoint` is `bp<N>`
- `block` is a bare `<block-id>`

Pause/error/revalidation events adapt `tcld_debug_state::DebugEvent`; target
lifecycle events are emitted by the v2 nub scheduler. `nub_started` and
`nub_stopped` exist only in the wire
vocabulary (they describe the connection, not the engine); the current
runtime does not emit them.

## Error shape

An `err` frame matches the failing request's serial. Payload
(`src/error.rs`):

```
code <enum> message <text>
```

`code` is required and drawn from a closed set of nine; `message` is
free-form (defaults to empty if absent). Unknown keys in the payload
are ignored for forward compatibility.

```
bad_args | not_attached | not_stopped | not_found | unimplemented |
internal | disconnected | bad_handshake | protocol_violation
```

The nub maps Tcl-level errors to codes by matching the error message
(`dispatch_req` in `crates/tcld-dbg-ext/src/nub.rs`): `wrong # args`
→ `bad_args`, `not paused` / `not Stopped` → `not_stopped`,
`not attached` → `not_attached`, `not found` → `not_found`, everything
else → `internal`.

## Handshake and capability negotiation

The nub always initiates. On TCP connect it sends `hello` (serial 0):

```
nub_version <semver> tcl_version <patchlevel> protocol_version 2 capabilities {<list>}
```

The controller replies with `hello_ack` (serial 0):

```
controller_version <semver> protocol_version 2 capabilities {<list>}
```

Rules, as implemented:

- Field order above is the canonical emission order (what the code
  writes). Parsers are order-insensitive dicts.
- Required fields: `hello` needs `nub_version` and `protocol_version`;
  `hello_ack` needs `controller_version` and `protocol_version`.
  Missing (or zero) `protocol_version` is a parse error.
- Unknown fields in either payload are ignored silently for compatible v2
  additions.
- `protocol_version` mismatch is a hard close: the Rust controller
  returns `ControllerError::HandshakeMismatch` and exchanges no
  further frames; the Tcl controller raises and closes.
- The controller applies a 5-second handshake timeout
  (`HANDSHAKE_TIMEOUT` in `src/controller.rs`; socket read/write
  timeouts during the handshake, cleared once `Open`).
- Any first frame other than `hello` fails the handshake
  (`ControllerError::UnexpectedFrame`).
- `capabilities` is a list of opt-in feature tokens. v2 currently ships the
  empty list on both sides; unknown tokens must be accepted silently.

## Connection state machine

Controller (`ControllerState` in `src/controller.rs`):

```
Idle → Listening → Accepting → Handshaking → Open → Closing → Idle
```

Nub (`::tcld::dbg::__nub__::state`, plus `::tcld::dbg::nub_status`):

```
disconnected → connecting → handshaking → open → closing → disconnected
```

- Sending an op while the controller is not `Open` fails synchronously
  with `ControllerError::NotConnected` (Rust) or a
  `not_connected: …` Tcl error — no wire I/O, no blocking.
- On the nub, `send_frame` is a no-op unless the state is `open` (or
  the frame is the `hello` itself); a socket-write failure tears the
  connection down to `disconnected` without crashing the interp — the
  debugged program keeps running if the controller dies.
- Either side may send `bye` at any time after the handshake; the
  receiver closes without replying.

### Running vs paused: the nub's two read modes

While the debugged script runs, the nub reads frames via `fileevent`
(event-loop integrated, non-blocking). When the engine pauses (a
breakpoint or step lands), the nub enters a synchronous **pause loop**
(`pause_loop` in `crates/tcld-dbg-ext/src/nub.rs`): `fileevent` is
disabled and frames are read with blocking I/O so that the user's
paused frame stays on the Tcl stack while ops dispatch — that is what
makes `uplevel`-based ops (`get_var`, `eval_in_frame`, …) see the
user's locals. A `step_into` / `step_over` / `step_out` / `continue`
request is dispatched, answered, and then exits the pause loop,
resuming execution. This mirrors TclPro's `DbgNub_Wait` /
`DbgNub_HandleEvent` structure.

## Request/response matching

- The controller assigns serials from a monotonically increasing
  `u64` counter starting at 1. Every `req` gets a fresh serial; the
  matching `resp` or `err` carries the same serial.
- v2 is single-in-flight by construction: `Controller::send` blocks
  until its serial's response arrives (default timeout 30 s,
  `DEFAULT_RESPONSE_TIMEOUT`) — but the schema keys responses by
  serial, so concurrent RPC can land later without a wire change
  (`gated:concurrent-rpc`).
- `evt` frames may interleave with responses on the wire in either
  order. The Rust controller's reader thread demultiplexes by `kind`
  into a per-serial response map and a FIFO event queue; consume
  events via `drain_events()` (non-blocking) or `next_event(timeout)`
  (blocking). The Tcl controller does the same with a pending-response
  array and an event list, plus optional per-kind callbacks
  (`::tcld::ctrl::on_event <kind> <script>`, kind `all` matches every
  event).
- Event order on the wire equals emission order in the nub (TCP is
  in-order; there is one stream).

## Spawning / attaching a nub

Protocol v2 uses one topology: the controller listens and the nub connects.

Controller side (Rust):

1. `Controller::listen("127.0.0.1:0".parse()?)` — bind; port 0 lets
   the OS pick; the bound port is returned.
2. `controller.spawn_nub(tclsh, bootstrap, dylib)` —
   spawns `tclsh <bootstrap>` with the environment the bootstrap
   script consumes:

   | Env var | Meaning |
   |---|---|
   | `TCLD_DBG_DYLIB` | absolute path to `libtcld_dbg.{dylib,so}` |
   | `TCLD_NUB_HOST` | controller listener host (`127.0.0.1`) |
   | `TCLD_NUB_PORT` | controller listener port |
3. `controller.accept()` — blocks until the nub connects and the
   handshake completes; returns the nub's capabilities.
4. Stage breakpoints and watches, then send
   `start_target path stop_on_entry`. Target execution begins with this request.
5. Drive ops with `send(op)`; consume events; `close()` sends `bye`,
   joins the reader thread, and returns to `Idle`.

`spawn_nub` returns a `NubChild` guard; dropping it kills the child if
it is still alive (up to 5 s of grace for voluntary exit, then a hard
kill) so tests and demos cannot orphan `tclsh` processes.

Nub side (the bootstrap,
`crates/tcld-dbg-ext/scripts/tcld_nub_bootstrap.tcl`):

```tcl
load $env(TCLD_DBG_DYLIB) Tcld_dbg
::tcld::dbg::engine_install
::tcld::dbg::install_command_wrappers
::tcld::dbg::nub_connect $env(TCLD_NUB_HOST) $env(TCLD_NUB_PORT)
# The bootstrap loop waits Ready and executes queued start_target requests.
```

Any already-running tclsh can attach itself to a listening controller
by running those same commands — `::tcld::dbg::nub_connect <host>
<port>` performs the connect + handshake and installs the event
forwarder. `::tcld::dbg::nub_status` reports the nub-side state;
`::tcld::dbg::nub_disconnect` sends `bye` and tears down.

The Tcl controller (`crates/tcld-dbg-ext/scripts/tcld_ctrl.tcl`,
package `tcld_ctrl`) exposes the same flow to pure-Tcl frontends:
`::tcld::ctrl::listen <port>`, `accept`, `send <op> ?args…?`,
`drain_events`, `next_event`, `on_event`, `status`, `close`.

## Verified usage modes

The Tier 4 test suite exercises the same engine surface in three
modes and asserts identical observable behavior:

| Mode | What runs where | Verifier |
|---|---|---|
| **in-process** | `::tcld::dbg::*` invoked directly in the debugged interp; no wire | Tier 0–3 suites (`live_engine.rs`, `tier1_wrappers.rs`, `tier2_surface.rs`, `tier3_features.rs`, `tclpro_parity.rs`) plus the loopback bridge test `wire_nub_loopback.rs` |
| **remote-rust** | Rust `Controller` spawns a real `tclsh` nub subprocess, drives it over loopback TCP | `tcld-dbg-ext/tests/wire_real_tclsh.rs`, `wire_parity_matrix.rs` (both gated `--features integration_tclsh`) |
| **remote-tcl** | pure-Tcl `tcld_ctrl` controller drives a nub over loopback TCP | `tcld-dbg-ext/tests/wire_ctrl_tcl_loopback.rs` (two interps in one process), `scripts/tcld_ctrl_demo.tcl` (fully out-of-process, tclsh ↔ tclsh) |

## Forward compatibility (future protocol majors)

These are format reservations. Protocol v2 does not implement them.

| Reservation | Gate | v2 behavior |
|---|---|---|
| Authentication / encryption (`auth <method>` on `hello`, `auth-token` op) | unknown `hello` keys are ignored; `auth-token` is outside the closed op set |
| Compression (`compress <algo>`) | frames are uncompressed; envelope extensions require a new protocol major |
| Multiple simultaneous nubs | one connected nub per controller |
| UDS, WebSocket, or other transports | loopback TCP only |
| Concurrent in-flight requests | one request at a time; responses already carry serials |
| Async controller API | synchronous and blocking |

The capability list on `hello`/`hello_ack` (empty in v2) is the
negotiation point for all of these.

## Reference demos

- **Rust**: [`examples/wire_demo.rs`](examples/wire_demo.rs)

  ```
  cargo build -p tcld-dbg-ext            # produces libtcld_dbg.dylib
  cargo run -p tcld-debug-wire --example wire_demo
  ```

  Optional env: `TCLD_TCLSH` (path to a Tcl 9 `tclsh`),
  `TCLD_DBG_DYLIB` (path to the built dylib). Defaults: well-known
  Homebrew locations / `$PATH` for tclsh, `target/{debug,release}`
  for the dylib. Pass a user script as the first argument (defaults
  to `fixtures/demo.tcl`).

- **Tcl**: `crates/tcld-dbg-ext/scripts/tcld_ctrl_demo.tcl`

  ```
  tclsh9.0 crates/tcld-dbg-ext/scripts/tcld_ctrl_demo.tcl
  ```

Both demos start a nub, set a breakpoint, wait for `paused`, and inspect
variables. The Rust demo also steps, reads the updated variable, and continues
to a clean nub exit. The Rust and Tcl controllers emit identical frames for the
same operation sequence.

## Protocol limits

- Handshake fields have a canonical emission order, while parsers accept any
  field order.
- The frame envelope has exactly three elements. Compatible additions belong
  inside handshake, event, or error dictionaries; envelope changes require a
  new protocol major.
- Unknown operations currently map to `internal`. The `unimplemented` error
  code is reserved.
- `interrupt`, `bp_add`, `bp_list`, `wp_add`, and `proc_list` intentionally map
  to differently named engine commands. The wire names in this document are
  canonical.
- `nub_started` and `nub_stopped` are defined by the schema but are not emitted
  by the current runtime. A closed connection is reported through controller
  state.
- `run_to`, `register`, and `unregister` are in-process engine operations with
  no wire equivalents.
- `Controller::spawn_nub` accepts explicit interpreter, bootstrap, and library
  paths. Targets start later through `start_target`.
- Frames are limited to a 10-digit length prefix and a 32 MiB body.
- `step_over` and `step_out` retain the paused command's trace depth so their
  next pause is consistent across in-process, Rust-controller, and
  Tcl-controller execution.
