# Bundle gates

Bundles in `qa/gates/` run declarative regression gates through the same validation path as direct scenarios. Inspect project defaults under `scenario:` in `qa/config.yaml` and local overrides before choosing flags.

```bash
godot-qa scenario bundle list --project <project> --json
godot-qa scenario bundle run <name> --project <project> --json
```

Preserve existing warm/strict settings. To request them explicitly, use `--warm --warm-strict`; strict requires warm, either by flag or configuration. `--cold` overrides both defaults for intentional cold diagnosis; `--no-warm-strict` disables only strict. Neither is a fix for a failing regression gate.

## Warm execution and diagnosis

- Headless-warm scenarios reuse pooled headless sessions. Media-only scenarios use a serialized windowed session for screenshots, visual assertions, single-viewport layout audits, and audio capture.
- Structural cold reasons are declared `execution.isolation: cold`, per-scenario `godot_args`, multiple `viewports`, and fixed FPS incompatible with the warm session. Structural reasons take precedence over media needs.
- `warm_fallback_forced_cold` fails a strict run for `godot_args`, `multi_viewport`, `fixed_fps_mismatch`, `frame_pacing_invalid`, or `warm_transport_failure`. Inspect the child's failure and `data.warm.reason`; correct unintended incompatibility rather than relaxing the gate. An explicit cold declaration is exempt, but should express a real isolation requirement.
- The initial headless session failing to start fails strict immediately with `warm_session_start_failed`. Without strict, execution falls back to cold and reports the cause in `data.warm.start_failure`. Later session acquisition/start failures (`windowed_session_unavailable`, `warm_session_unavailable`) are infrastructure fallbacks exempt from scenario strict violations. Pool failures also appear in `data.warm_pool` and startup warnings.
- If resolved workers cover every runnable scenario, warm reuse is skipped (`no_reuse_below_parallelism`). Structural strict checks still run; `data.warm.strict_structural_check` records that enforcement. Do not interpret cold execution alone as a strict violation. `--equivalence-check` retains its warm leg and compares against the authoritative cold result.

Child `result.json` and bundle summaries record whether each scenario actually ran warm, its mode, and its fallback reason. Use those artifacts to distinguish scenario incompatibility, infrastructure failure, and deliberate isolation.

## Parallelism, selection, and scheduling

`--jobs` overrides `scenario.jobs`; the default is `0` (available parallelism). Positive values request workers; negative values reserve threads (`-2` on eight threads gives six). Workers are capped to the runnable child count. Separate Godot processes share one `user://` directory: consuming projects must isolate scratch paths by process/session instead of using fixed shared filenames.

`--only <scenario>` is repeatable. `--failed-only` selects failures from the newest run's summary; together the filters intersect. Repeated failed-only runs narrow the baseline, and an unfiltered run resets it. Unknown names or missing previous summaries fail before execution; an empty selection runs nothing successfully with a warning, so it is not evidence that the whole gate passed.

`--schedule manifest|longest-first` overrides `scenario.schedule`, defaulting to `longest-first`. Parallel scheduling uses recent history, dispatching unknown-cost scenarios first, then predicted slow ones. One worker or no history retains expansion order. Reported results remain in expansion order regardless of dispatch order.

Several bundle names in one invocation share one warm pool; the reuse decision uses their summed runnable count. Results contain `data.bundles[]` and an aggregate summary. Multiple names cannot be combined with `--equivalence-check`, `--only`, or `--failed-only`, and duplicate names fail validation.

`--retry-startup <count>` with optional `--retry-startup-backoff-ms <ms>` is bounded and opt-in, for infrastructure-only failures before gameplay assertions. It does not retry assertion, gameplay, or artifact failures.

## Evidence

Follow returned artifact paths. Each bundle rotates `.qa/artifacts/bundles/<bundle>/latest` to its newest run. Inspect `summary.json`, the artifact index, and relevant child results; filtered successes must be reported as filtered coverage. Warm runs preserve per-scenario log slices; missing markers can indicate an older copied addon. Keep CLI and addon versions aligned.

Optional deeper contract: the scenario-bundle sections of `docs/cli-spec.md` and `docs/scenario-format.md` in a matching godot-qa checkout or distribution.
