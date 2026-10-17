// Extracted from lib.rs; included by lib.rs to preserve the original crate namespace.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveProfileRetryPolicy {
    pub max_attempts: usize,
    pub backoff_millis: &'static [u64],
}

impl Default for LiveProfileRetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: LIVE_PROFILE_RETRY_BACKOFF_MILLIS.len() + 1,
            backoff_millis: LIVE_PROFILE_RETRY_BACKOFF_MILLIS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveProfileAttempt {
    pub attempt: usize,
    pub retryable: bool,
}

fn run_live_profile_protocol_command(
    config: &ProjectConfig,
    command: &str,
    record: &SessionRecord,
    params: Map<String, Value>,
) -> CommandResult {
    let policy = LiveProfileRetryPolicy::default();
    run_live_profile_protocol_command_with_request(
        config,
        command,
        record,
        params,
        policy,
        |params| send_live_request(config, record, SESSION_PROFILE, params, None),
    )
}

fn run_live_profile_protocol_command_with_request<F>(
    config: &ProjectConfig,
    command: &str,
    record: &SessionRecord,
    params: Map<String, Value>,
    policy: LiveProfileRetryPolicy,
    mut request: F,
) -> CommandResult
where
    F: FnMut(Map<String, Value>) -> Result<Value, FailureDetail>,
{
    let mut attempt = 1;
    let mut retry = LiveProfileRetryState::new(&policy);
    let sample_window = requested_profile_sample_window(&params);
    loop {
        let response = match request(params.clone()) {
            Ok(response) => response,
            Err(failure) => {
                let classified =
                    classify_live_profile_transport_failure(config, record, command, failure);
                if classified.attempt.retryable && attempt < policy.max_attempts {
                    let (backoff_ms, wait_aware) =
                        live_profile_retry_backoff_ms(&classified.failure, &policy, attempt);
                    retry.record_retryable_failure(
                        &classified.failure,
                        attempt,
                        backoff_ms,
                        wait_aware,
                    );
                    sleep_live_profile_backoff(backoff_ms);
                    attempt += 1;
                    continue;
                }
                retry.record_final_failure(&classified.failure.kind, attempt);
                return live_profile_failure_result(
                    command,
                    classified.failure,
                    retry.metadata("failure", &sample_window),
                    sample_window,
                );
            }
        };
        if let Err(error) = validate_response(command, &response) {
            retry.record_final_failure(&error.failure.kind, attempt);
            return live_profile_failure_result(
                command,
                error.failure,
                retry.metadata("malformed_response", &sample_window),
                sample_window,
            );
        }
        let object = response.as_object().unwrap_or(&Map::new()).clone();
        if object.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            let mut data = object
                .get("data")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            if let Some(metadata) = retry.metadata("success", &sample_window) {
                data.insert("retry".to_string(), metadata);
            }
            return CommandResult {
                ok: true,
                command: command.to_string(),
                message: "live command completed".to_string(),
                data,
                failures: Vec::new(),
                exit_code: 0,
            };
        }
        let error_data = object
            .get("error")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let classified = classify_live_profile_runtime_failure(
            command,
            &error_data,
            LiveProfileAttempt {
                attempt,
                retryable: false,
            },
        );
        if classified.attempt.retryable && attempt < policy.max_attempts {
            let (backoff_ms, wait_aware) =
                live_profile_retry_backoff_ms(&classified.failure, &policy, attempt);
            retry.record_retryable_failure(&classified.failure, attempt, backoff_ms, wait_aware);
            sleep_live_profile_backoff(backoff_ms);
            attempt += 1;
            continue;
        }
        retry.record_final_failure(&classified.failure.kind, attempt);
        return live_profile_failure_result(
            command,
            classified.failure,
            retry.metadata("failure", &sample_window),
            sample_window,
        );
    }
}

#[derive(Debug, Clone)]
struct LiveProfileRetryState {
    attempts: usize,
    max_attempts: usize,
    backoff_millis: Vec<u64>,
    retryable_failure_kinds: Vec<String>,
    observed_retryable_failures: Vec<Value>,
    wait_aware: bool,
    final_status: Option<String>,
}

impl LiveProfileRetryState {
    fn new(policy: &LiveProfileRetryPolicy) -> Self {
        Self {
            attempts: 1,
            max_attempts: policy.max_attempts,
            backoff_millis: Vec::new(),
            retryable_failure_kinds: Vec::new(),
            observed_retryable_failures: Vec::new(),
            wait_aware: false,
            final_status: None,
        }
    }

    fn record_retryable_failure(
        &mut self,
        failure: &FailureDetail,
        attempt: usize,
        backoff_ms: u64,
        wait_aware: bool,
    ) {
        self.retryable_failure_kinds.push(failure.kind.to_string());
        self.backoff_millis.push(backoff_ms);
        self.wait_aware |= wait_aware;
        self.observed_retryable_failures.push(json!({
            "attempt": attempt,
            "retry_after_ms": backoff_ms,
            "wait_aware": wait_aware,
            "type": failure.kind,
            "command": failure.command,
            "message": failure.message,
            "actual": failure.actual,
            "details": failure.details.clone(),
        }));
        self.attempts += 1;
    }

    fn record_final_failure(&mut self, kind: &str, attempt: usize) {
        self.attempts = attempt;
        self.final_status = Some(kind.to_string());
    }

    fn metadata(&self, final_status: &str, sample_window: &Value) -> Option<Value> {
        if self.attempts <= 1 {
            return None;
        }
        Some(json!({
            "attempts": self.attempts,
            "max_attempts": self.max_attempts,
            "backoff_milliseconds": self.backoff_millis,
            "wait_aware": self.wait_aware,
            "retryable_failure_kinds": LIVE_PROFILE_RETRYABLE_FAILURE_KINDS,
            "observed_retryable_failure_kinds": self.retryable_failure_kinds,
            "observed_retryable_failures": self.observed_retryable_failures,
            "final_status": self.final_status.as_deref().unwrap_or(final_status),
            "exhausted": self.attempts >= self.max_attempts && final_status != "success",
            "sample_window": sample_window,
        }))
    }
}

fn live_profile_retry_backoff_ms(
    failure: &FailureDetail,
    policy: &LiveProfileRetryPolicy,
    attempt: usize,
) -> (u64, bool) {
    if let Some(backoff_ms) = live_command_busy_backoff_ms(failure) {
        return (backoff_ms, true);
    }
    (
        policy
            .backoff_millis
            .get(attempt.saturating_sub(1))
            .copied()
            .unwrap_or(50),
        false,
    )
}

fn sleep_live_profile_backoff(milliseconds: u64) {
    if milliseconds > 0 {
        thread::sleep(Duration::from_millis(milliseconds));
    }
}

fn requested_profile_sample_window(params: &Map<String, Value>) -> Value {
    json!({
        "frames": params.get("frames").cloned().unwrap_or(Value::Null),
        "seconds": params.get("seconds").cloned().unwrap_or(Value::Null),
    })
}

#[derive(Debug, Clone)]
struct ClassifiedLiveProfileFailure {
    failure: FailureDetail,
    attempt: LiveProfileAttempt,
}

fn live_profile_failure_result(
    command: &str,
    failure: FailureDetail,
    retry: Option<Value>,
    sample_window: Value,
) -> CommandResult {
    let mut data = Map::from_iter([("sample_window".to_string(), sample_window)]);
    if let Some(retry) = retry {
        data.insert("retry".to_string(), retry);
    }
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: failure
            .message
            .clone()
            .unwrap_or_else(|| "live session profile failed".to_string()),
        data,
        failures: vec![failure],
        exit_code: 1,
    }
}

fn classify_live_profile_transport_failure(
    config: &ProjectConfig,
    record: &SessionRecord,
    command: &str,
    failure: FailureDetail,
) -> ClassifiedLiveProfileFailure {
    if failure.kind == "session_connect_failed" {
        let attempt = LiveProfileAttempt {
            attempt: 1,
            retryable: false,
        };
        let metadata_path = session_metadata_path(config, record);
        let failure =
            classify_session_connect_failure(record, command, &metadata_path, failure);
        if matches!(
            failure.kind.as_str(),
            "stale_session" | "android_device_unavailable"
        ) {
            ClassifiedLiveProfileFailure { failure, attempt }
        } else {
            classify_live_profile_failure(command, failure)
        }
    } else {
        classify_live_profile_failure(command, failure)
    }
}

fn classify_live_profile_runtime_failure(
    command: &str,
    error_data: &Map<String, Value>,
    attempt: LiveProfileAttempt,
) -> ClassifiedLiveProfileFailure {
    let failure = failure_detail_from_live_error(command, error_data);
    classify_live_profile_failure_with_attempt(command, failure, attempt)
}

fn classify_live_profile_failure(
    command: &str,
    failure: FailureDetail,
) -> ClassifiedLiveProfileFailure {
    classify_live_profile_failure_with_attempt(
        command,
        failure,
        LiveProfileAttempt {
            attempt: 1,
            retryable: false,
        },
    )
}

fn classify_live_profile_failure_with_attempt(
    command: &str,
    mut failure: FailureDetail,
    attempt: LiveProfileAttempt,
) -> ClassifiedLiveProfileFailure {
    failure.command = Some(command.to_string());
    let mut classified_attempt = attempt;
    match failure.kind.as_str() {
        "invalid_session_token" | "session_token_invalid" => {
            failure.kind = "session_token_invalid".to_string();
        }
        "session_connect_failed" => {
            if failure.actual.as_ref().is_some_and(is_timeout_actual) {
                failure.kind = "session_connect_timeout".to_string();
            } else if failure_text_contains(&failure, "Resource temporarily unavailable")
                || failure_text_contains(&failure, "os error 11")
            {
                classified_attempt.retryable = true;
            } else {
                failure.kind = "session_unreachable".to_string();
            }
        }
        "resource_unavailable" | "resource_busy" | "bridge_busy" | "session_bridge_busy" => {
            failure.kind = "session_bridge_busy".to_string();
            classified_attempt.retryable = true;
        }
        _ => {}
    }
    failure
        .details
        .insert("retryable".to_string(), json!(classified_attempt.retryable));
    failure
        .details
        .insert("attempt".to_string(), json!(classified_attempt.attempt));
    ClassifiedLiveProfileFailure {
        failure,
        attempt: classified_attempt,
    }
}
