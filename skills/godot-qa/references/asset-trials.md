# Verified asset trials

Use this workflow when the task calls for trying source media in an existing saved scene and checking its binding and gameplay behavior. It changes that scene until the trial is kept or discarded. Prefer an existing project manifest and validation scenario; read `asset status --project <project> --json` first to discover an active reservation or interrupted trial.

## Supported bindings and preparation

Version 1 supports PNG textures on `Sprite2D.texture` or `TextureRect.texture`, and PCM16 mono/stereo WAV streams on `AudioStreamPlayer.stream` or `AudioStreamPlayer2D.stream`. PNGs need lossless imports and are limited to 4096×4096; WAV duration must be positive and at most 30 seconds. Both source formats have a 32 MiB limit. Inherited edited scenes and edited targets inside nested instances are unsupported.

Close external editors before starting. Choose absent `res://` destinations with existing parent directories; preserve original assets. Evidence under the project resource tree must already be excluded from Godot import with `.gdignore`, or use an artifact root outside the project. Relative paths, including the manifest path, resolve against the selected project.

A texture-only manifest, with project-specific paths and targets substituted:

```yaml
format: godot-qa.asset-trial.v1
scene: res://items/pickup.tscn
validation:
  scenario: pickup_review
bindings:
  - id: pickup_texture
    source: qa/candidates/pickup.png
    destination: res://art/pickup_candidate.png
    edit_path: Sprite
    runtime_target: qa:pickup_sprite
    property: texture
    evidence:
      snapshot: pickup_visible
      state_before: pickup_before
      state_after: pickup_after
```

`validation` selects exactly one `scenario` or `scenario_file`. Each binding has a unique `id` and exactly one `snapshot` or `audio` evidence name. `edit_path` is relative to the edited scene root; `runtime_target` resolves the instance exercised by validation. Editing a shared scene affects every instance, while evidence verifies only the selected instance.

## Validation evidence

The validation scenario declares `determinism.seed`, `determinism.fixed_fps`, `determinism.locale`, one positive `viewport`, and explicit `project_args` (use `[]` when unused). It needs a gameplay assertion beyond clean errors, layout, or screenshot matching. For texture evidence, enable `snapshots.save_screenshot` and keep `screenshot_match` off the selected snapshot: changed artwork is expected.

Each selected capture must appear exactly once, immediately between the manifest's named state records. Example steps using an existing read-only provider:

```yaml
steps:
  - record_state:
      name: pickup_before
      source: provider
      provider: pickup_state
  - snapshot: pickup_visible
  - record_state:
      name: pickup_after
      source: provider
      provider: pickup_state
assert:
  - state_matches:
      source: provider
      provider: pickup_state
      path: available
      operator: equals
      expected: true
```

Record gameplay state that should remain comparable between baseline and candidate runs. These records preserve observations; assertions still establish expected behavior. Use real input for the behavior being tested, with fixtures limited to setup.

For a WAV binding, use `property: stream` and `evidence.audio: <capture-name>`, bracket an `audio_capture` step with the same state-record pattern, and capture the selected player's bus while the candidate is actually playing. Merely calling playback does not prove captured audio. The trial requires observed playback, a real audio driver, and non-silent evidence; its deterministic validation does not certify realtime audio-clock synchronization.

## Run, review, and recover

```bash
godot-qa asset try qa/asset-trials/pickup.yaml --project <project> --json
godot-qa asset status <trial-id> --project <project> --json
```

`try` captures baseline evidence, imports candidates, applies scene bindings through Godot, and validates in a fresh process. Follow returned paths and review `technical_status`, `review_status`, failures, and before/after evidence separately. A technical pass leaves visual/audio review unapproved.

Complete the user's chosen disposition with `asset keep <trial-id> --project <project> --json` or `asset discard <trial-id> --project <project> --json`. Keep requires a passing trial and rechecks its inputs and evidence. Discard restores the original scene and also recovers interrupted trials; retain `.qa/asset-trials/<trial-id>/` and its backup until recovery is complete. Neither operation approves screenshot baselines.

An active reservation blocks another trial, import, source writes, watch/hot reload, and addon installation. Ordinary previews are allowed during idle review, but stop them before keep/discard. Resolve an existing trial through its reported recovery guidance instead of deleting its reservation or journal. Discard may retain changed or possibly referenced media with warnings; inspect those paths or choose new destinations before retrying.

Optional deeper contracts in a matching godot-qa checkout/distribution: `docs/cli-spec.md` (verified asset trials), `docs/scenario-format.md` (asset trial validation scenarios), and `examples/sample_project/qa/asset-trials/coin.yaml`.
