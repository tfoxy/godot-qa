# Security Model

The runtime bridge can inspect and control a running Godot project. Treat it as a local developer tool, not a production service.

## Defaults

- Enabling the plugin may register a dormant autoload singleton, but runtime QA stays disabled unless test mode is enabled.
- Test mode is enabled only by `--qa`, `--qa-*`, or an explicit `godot_qa/enabled` project setting.
- Binds to `127.0.0.1` only.
- Requires a random per-session token.
- Stores singleton tokens in `.qa/session.token` and pooled tokens in each isolated session artifact directory's `session.token`; normal CLI output and session metadata keep only the token file path.
- Does not expose arbitrary script execution.
- Writes artifacts only under configured artifact directories.
- Excluded from production exports by default.

## Threats

### Unauthorized local control

A local process might send commands to the bridge.

Mitigations:

- Random token.
- On Unix, token files are created exclusively with owner-only (`0600`) permissions and atomically renamed into place, so new secrets are never briefly group/world-readable. Replacing a token repairs older file permissions; reading an existing token also restricts its permissions before reading. Windows retains its existing filesystem/ACL behavior.
- Desktop file-transport fallback requests contain the token and use the same owner-only creation, including temporary files, before atomic publication. Completed or timed-out requests are removed.
- The hot-reload debug host reuses the session token. The CLI publishes it owner-only before spawning the sidecar; the sidecar reads `session.token` beside its metadata. Every control request, including status/ping, carries the token and is authenticated before dispatch. Absent or incorrect credentials return `unauthorized_control_request` and send no debugger frames. Tokens never appear in sidecar argv, metadata, or logs.
- Debug-host control requests are limited to 1 MiB (including the newline) and a two-second total read deadline. Oversized, incomplete, or timed-out messages are closed without dispatch. Control replies are capped at 1 MiB and a ten-second total client read deadline. Game and control sockets use two-second write timeouts, and debugger frame batches have a two-second total write deadline. A terminal game disconnect or write failure ends the sidecar and releases its listeners; a pending control read is bounded by its deadline.
- Port is random by default.
- Only localhost binding.
- Every live command is validated again inside Godot; the CLI is not trusted as the final authority.

### Production exposure

QA bridge accidentally ships in release builds.

Mitigations:

- Document export exclusion.
- Runtime checks for test flags.
- Optional project setting to refuse release mode.

### Unsafe live edits

Live editing can create source changes that do not reflect real editor state.

Mitigations:

- Live edits are temporary by default.
- Source patch export is explicit.
- Live mutation is limited to a narrow allowlist of reviewed properties and methods.
- Patch application uses Godot APIs (`PackedScene` + `ResourceSaver`) rather than raw `.tscn` string surgery.
- Live-only method calls are never auto-exported into source patches.

### Arbitrary code execution

Unsafe eval commands could execute malicious code.

Mitigations:

- Do not implement eval in MVP.
- If added, name it `eval --unsafe`, require explicit config, and never enable in CI by default.

## Android live-session boundary

- Only Android debug template builds inspect the fixed private bootstrap file.
  Release builds, editor runs, desktop builds, and implicit/missing bootstrap
  remain inactive.
- The session token is streamed over stdin into app-private storage. It is never
  placed in process argv, Intent extras, logs, host request artifacts, or
  persisted session metadata. Malformed bridge input is reported without
  echoing the raw request.
- The runtime bridge still binds only to `127.0.0.1`; the CLI creates and later
  removes only the recorded session forward.
- The runtime derives `user://godot_qa/live/<session-id>` itself. Returned device
  paths are untrusted and must be strictly beneath that root before a binary-safe
  `run-as` pull and atomic host write. Relative suffix segments containing `:`
  are rejected to prevent Windows drive-relative paths or alternate data streams.
- Stop and rollback scope cleanup to the recorded package, exact forward,
  bootstrap file, private session directory, and logcat helper. Device
  unavailability retains metadata so later cleanup remains recoverable.
- Windows host logcat cleanup pairs `logcat_pid` with optional
  `logcat_process_identity` creation metadata. Missing or mismatched identity
  prevents fallback termination and reports `android_logcat_stop_failed`.
- Wireless ADB must already be authorized and configured. The CLI may reconnect
  the selected `host:port` or mDNS service endpoint after export but does not
  pair devices.
