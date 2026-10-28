use super::*;

const DEFAULT_MAX_BYTES: u64 = 512 * 1024 * 1024;
const HARD_MAX_BYTES: u64 = 1024 * 1024 * 1024;

pub(super) struct BurstCapture {
    pub frames: Vec<LiveScreenshotCapture>,
    pub failures: Vec<FailureDetail>,
    pub metadata: Map<String, Value>,
}

pub(super) fn validate(
    command: &str,
    options: &ScreenshotSequenceOptions,
) -> Result<(), CommandResult> {
    let invalid = match options.capture_mode.as_str() {
        "serial" if options.max_bytes.is_some() || options.max_seconds.is_some() => {
            Some("--max-bytes and --max-seconds require --capture-mode burst")
        }
        "serial" => None,
        "burst"
            if options
                .max_bytes
                .is_some_and(|n| n == 0 || n > HARD_MAX_BYTES) =>
        {
            Some("burst --max-bytes must be between 1 and 1073741824")
        }
        "burst"
            if options
                .max_seconds
                .is_some_and(|n| !n.is_finite() || n <= 0.0 || n > 10.0) =>
        {
            Some("burst --max-seconds must be finite, positive, and at most 10")
        }
        "burst" => None,
        _ => Some("--capture-mode must be serial or burst"),
    };
    if let Some(message) = invalid {
        return Err(screenshot_args_invalid(
            command,
            "screenshot sequence failed",
            "screenshot_sequence_args_invalid",
            message,
            json!({"capture_mode":options.capture_mode,"max_bytes":options.max_bytes,"max_seconds":options.max_seconds}),
            json!("valid burst limits, or serial without burst limits"),
        ));
    }
    Ok(())
}

pub(super) fn capture(
    config: &ProjectConfig,
    command: &str,
    options: &ScreenshotSequenceOptions,
    plan: &ScreenshotSequencePlan,
) -> Result<BurstCapture, CommandResult> {
    let result = runtime::run_live_screenshot_protocol_command(
        config,
        command,
        runtime::SCREENSHOT_BURST,
        Map::from_iter([
            (
                "name".to_string(),
                json!(format!(
                    "burst_{}",
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos()
                )),
            ),
            ("timing_mode".to_string(), json!(plan.mode)),
            ("capture_frames".to_string(), json!(plan.capture_frames)),
            ("seconds".to_string(), json!(plan.seconds)),
            ("fps".to_string(), json!(plan.fps)),
            (
                "max_bytes".to_string(),
                json!(options.max_bytes.unwrap_or(DEFAULT_MAX_BYTES)),
            ),
            (
                "max_seconds".to_string(),
                json!(options.max_seconds.unwrap_or(3.0)),
            ),
            (
                "state_guards".to_string(),
                options.state_guards.requirements_json(),
            ),
            (
                "stop_after_state".to_string(),
                json!(options.stop_after_state),
            ),
        ]),
    );
    if !result.ok {
        return Err(result);
    }
    decode_response(command, plan, options, result.data)
}

fn decode_response(
    command: &str,
    plan: &ScreenshotSequencePlan,
    options: &ScreenshotSequenceOptions,
    mut metadata: Map<String, Value>,
) -> Result<BurstCapture, CommandResult> {
    let invalid = |message: &str| {
        protocol_error(
            command,
            "screenshot burst failed",
            message,
            json!("bounded, complete burst response or explicit partial failure"),
            Value::Object(metadata.clone()),
        )
    };
    let complete = metadata
        .get("complete")
        .and_then(Value::as_bool)
        .ok_or_else(|| invalid("burst response requires a boolean complete field"))?;
    let frame_count = metadata
        .get("frames")
        .and_then(Value::as_array)
        .map(Vec::len)
        .ok_or_else(|| invalid("burst response requires frames"))?;
    let failure_values = metadata
        .get("failures")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("burst response requires failures"))?;
    let stopped = metadata
        .get("stop_condition_matched")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if metadata.get("capture_mode").and_then(Value::as_str) != Some("burst")
        || frame_count > 96
        || frame_count > plan.capture_frames.len()
        || (stopped && options.stop_after_state.is_none())
        || (complete && stopped && frame_count == 0)
        || (!complete && failure_values.is_empty())
        || (complete
            && (!failure_values.is_empty()
                || (frame_count != plan.capture_frames.len() && !stopped)))
    {
        return Err(invalid(
            "burst completion and frame counts are inconsistent",
        ));
    }
    let captured_in_memory = metadata
        .get("captured_in_memory_count")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid("burst response requires captured_in_memory_count"))?;
    let samples = metadata
        .get("stop_condition_sample_count")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid("burst response requires stop_condition_sample_count"))?;
    if captured_in_memory > 96
        || captured_in_memory < frame_count as u64
        || (complete && captured_in_memory != frame_count as u64)
        || (stopped && captured_in_memory == 0)
        || (options.stop_after_state.is_some()
            && (samples > captured_in_memory || (complete && samples != captured_in_memory)))
        || (options.stop_after_state.is_none() && samples != 0)
    {
        return Err(invalid(
            "burst stop sampling and captured counts are inconsistent",
        ));
    }
    if complete && stopped {
        let at = metadata.get("stop_condition_stopped_at");
        if at
            .and_then(|value| value.get("sequence_index"))
            .and_then(Value::as_u64)
            != Some(frame_count as u64 - 1)
            || at
                .and_then(|value| value.get("index"))
                .and_then(Value::as_u64)
                != Some(plan.capture_frames[frame_count - 1])
        {
            return Err(invalid(
                "burst stop position must identify the final captured frame",
            ));
        }
    }
    let required_guards =
        crate::assertion_proof::definition_identity(&options.state_guards.requirements_json());
    let guard_count = options
        .state_guards
        .require_visible
        .iter()
        .chain(options.state_guards.require_hidden.iter())
        .filter(|target| !target.trim().is_empty())
        .count()
        + usize::from(options.state_guards.require_state.is_some());
    let mut previous_elapsed = 0_u64;
    let mut previous_draw = None;
    for (index, frame) in metadata
        .get("frames")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .enumerate()
    {
        let draw = frame.get("draw_ordinal").and_then(json_u64);
        if frame.get("index").and_then(json_u64) != Some(plan.capture_frames[index])
            || draw.is_none()
            || previous_draw
                .zip(draw)
                .is_some_and(|(previous, next)| next <= previous)
            || frame.get("engine_frame").and_then(json_u64).is_none()
            || frame.get("time_msec").and_then(json_u64).is_none()
            || frame.get("draw_forced").and_then(Value::as_bool) != Some(false)
        {
            return Err(invalid(
                "burst frame is missing authentic capture metadata or has invalid ordering",
            ));
        }
        let elapsed = frame.get("elapsed_usec").and_then(json_u64);
        let guard = frame.get("state_guards");
        let checks = guard
            .and_then(|guard| guard.get("checks"))
            .and_then(Value::as_array);
        if elapsed.is_none_or(|value| {
            value < previous_elapsed
                || (complete && (value as f64) >= options.max_seconds.unwrap_or(3.0) * 1_000_000.0)
        }) || frame.get("capture_source").and_then(Value::as_str) != Some("godot_viewport")
            || guard
                .and_then(|guard| guard.get("ok"))
                .and_then(Value::as_bool)
                != Some(true)
            || guard.and_then(|guard| guard.get("requirements_identity")) != Some(&required_guards)
            || checks.is_none_or(|checks| {
                checks.len() != guard_count
                    || checks.iter().any(|check| {
                        check
                            .get("result")
                            .and_then(|result| result.get("ok"))
                            .and_then(Value::as_bool)
                            != Some(true)
                    })
            })
        {
            return Err(invalid(
                "burst frame requires valid observed timing and successful requested guard evidence",
            ));
        }
        if let Some(fps) = plan.fps {
            let requested = (index as f64 / fps * 1_000_000.0).ceil() as u64;
            if frame.get("requested_time_usec").and_then(json_u64) != Some(requested)
                || elapsed.unwrap() < requested
            {
                return Err(invalid(
                    "burst time sample did not meet its monotonic deadline",
                ));
            }
        } else if draw != Some(plan.capture_frames[index]) {
            return Err(invalid(
                "burst frame draw ordinal must match the requested frame schedule",
            ));
        }
        previous_elapsed = elapsed.unwrap();
        previous_draw = draw;
    }
    let frame_values = metadata
        .remove("frames")
        .and_then(|value| value.as_array().cloned())
        .ok_or_else(|| {
            protocol_error(
                command,
                "screenshot burst failed",
                "burst response must include frames",
                json!("frames array"),
                Value::Null,
            )
        })?;
    let failures: Vec<FailureDetail> = metadata
        .remove("failures")
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .map(|value| {
            serde_json::from_value::<FailureDetail>(value).unwrap_or_else(|error| FailureDetail {
                kind: "protocol_error".to_string(),
                command: Some(command.to_string()),
                message: Some(format!("invalid burst failure: {error}")),
                ..FailureDetail::default()
            })
        })
        .collect();
    let mut frames = Vec::new();
    for value in frame_values {
        let data = value.as_object().cloned().ok_or_else(|| {
            protocol_error(
                command,
                "screenshot burst failed",
                "burst frame must be an object",
                json!("frame object"),
                value.clone(),
            )
        })?;
        frames.push(live_screenshot_capture_from_response(command, data)?);
    }
    Ok(BurstCapture {
        frames,
        failures,
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_responses_require_explicit_failures_and_valid_completion() {
        let options = super::super::tests::sequence_options();
        let plan = screenshot_sequence_plan("screenshot sequence", &options).unwrap();
        let valid = json!({"capture_mode":"burst", "complete":false, "frames":[], "captured_in_memory_count":0, "stop_condition_sample_count":0, "failures":[{"type":"screenshot_burst.memory_limit"}]}).as_object().unwrap().clone();
        assert!(decode_response("screenshot sequence", &plan, &options, valid.clone()).is_ok());
        for key in ["complete", "failures", "frames"] {
            let mut missing = valid.clone();
            missing.remove(key);
            assert!(decode_response("screenshot sequence", &plan, &options, missing).is_err());
        }
        let mut no_failure = valid.clone();
        no_failure.insert("failures".to_string(), json!([]));
        assert!(decode_response("screenshot sequence", &plan, &options, no_failure).is_err());
        let mut stop_options = options.clone();
        stop_options.stop_after_state = Some(json!({"path":"ready", "expected":true}));
        let stopped_write_failure = json!({"capture_mode":"burst", "complete":false, "frames":[], "captured_in_memory_count":1, "stop_condition_sample_count":1,"stop_condition_matched":true, "stop_condition_stopped_at":{"index":0,"sequence_index":0}, "failures":[{"type":"artifact_write_failed"}]}).as_object().unwrap().clone();
        let retained = decode_response(
            "screenshot sequence",
            &plan,
            &stop_options,
            stopped_write_failure,
        )
        .unwrap();
        assert_eq!(retained.failures[0].kind, "artifact_write_failed");
        let mut false_complete = valid;
        false_complete.insert("complete".to_string(), json!(true));
        false_complete.insert("failures".to_string(), json!([]));
        assert!(
            decode_response(
                "screenshot sequence",
                &plan,
                &options,
                false_complete.clone()
            )
            .is_err()
        );
        false_complete.insert("stop_condition_matched".to_string(), json!(true));
        assert!(decode_response("screenshot sequence", &plan, &options, false_complete).is_err());
    }

    #[test]
    fn burst_time_plans_reject_overflow_and_keep_monotonic_deadlines() {
        let mut options = super::super::tests::sequence_options();
        options.capture_mode = "burst".to_string();
        options.frames = None;
        options.every = None;
        options.seconds = Some(0.1);
        options.fps = Some(20.0);
        let plan = screenshot_sequence_plan("screenshot sequence", &options).unwrap();
        assert_eq!(plan.capture_frames, vec![0, 1, 2]);
        options.seconds = Some(f64::MAX);
        options.fps = Some(f64::MAX);
        assert!(screenshot_sequence_plan("screenshot sequence", &options).is_err());
    }

    #[test]
    fn completed_bursts_require_ordered_frames_and_requested_guard_evidence() {
        let options = super::super::tests::sequence_options();
        let plan = screenshot_sequence_plan("screenshot sequence", &options).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "godot-qa-burst-response-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("frame.png");
        RgbaImage::new(8, 8).save(&path).unwrap();
        let frames: Vec<Value> = plan.capture_frames.iter().map(|index| json!({
            "index":index,"draw_ordinal":index,"engine_frame":index+100,"time_msec":index+1000,"elapsed_msec":*index as f64,"elapsed_usec":index*1000,"capture_source":"godot_viewport","draw_forced":false,
            "artifacts":{"screenshot":path},"state_guards":{"ok":true,"requirements_identity":crate::assertion_proof::definition_identity(&options.state_guards.requirements_json()),"checks":[]}
        })).collect();
        let valid = json!({"capture_mode":"burst","complete":true,"captured_in_memory_count":3,"stop_condition_sample_count":0,"stop_condition_matched":false,"frames":frames,"failures":[]}).as_object().unwrap().clone();
        assert!(decode_response("screenshot sequence", &plan, &options, valid.clone()).is_ok());
        for mode in [
            "index",
            "draw_ordinal",
            "engine_frame",
            "elapsed_usec",
            "state_guards",
            "capture_source",
        ] {
            let mut changed = valid.clone();
            changed.get_mut("frames").unwrap()[1]
                .as_object_mut()
                .unwrap()
                .remove(mode);
            assert!(
                decode_response("screenshot sequence", &plan, &options, changed).is_err(),
                "accepted missing {mode}"
            );
        }
        let mut exceeded_deadline = valid.clone();
        exceeded_deadline.get_mut("frames").unwrap()[2]["elapsed_usec"] = json!(3_000_000);
        assert!(
            decode_response("screenshot sequence", &plan, &options, exceeded_deadline).is_err()
        );
        let mut wrong_guard = valid.clone();
        wrong_guard.get_mut("frames").unwrap()[0]["state_guards"]["ok"] = json!(false);
        assert!(decode_response("screenshot sequence", &plan, &options, wrong_guard).is_err());
        let mut wrong_order = valid;
        wrong_order
            .get_mut("frames")
            .unwrap()
            .as_array_mut()
            .unwrap()
            .swap(0, 1);
        assert!(decode_response("screenshot sequence", &plan, &options, wrong_order).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn burst_limits_are_explicit_and_bounded() {
        let mut options = super::super::tests::sequence_options();
        options.max_bytes = Some(1);
        assert!(validate("screenshot sequence", &options).is_err());
        options.capture_mode = "burst".to_string();
        assert!(validate("screenshot sequence", &options).is_ok());
        options.max_bytes = Some(HARD_MAX_BYTES + 1);
        assert!(validate("screenshot sequence", &options).is_err());
        options.max_bytes = None;
        options.max_seconds = Some(f64::INFINITY);
        assert!(validate("screenshot sequence", &options).is_err());
        options.max_seconds = Some(10.0);
        assert!(validate("screenshot sequence", &options).is_ok());
    }
}
