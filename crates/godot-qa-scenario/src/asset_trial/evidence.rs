use super::*;

pub(super) fn capability(result: &Value) -> Result<(), FailureDetail> {
    if !result["asset_trial"]["capabilities"]
        .as_array()
        .is_some_and(|a| a.iter().any(|v| v == "asset_trial_v1"))
    {
        return Err(fail(
            "addon_capability_missing",
            "asset_trial_v1",
            "installed addon must support the complete writer, probe, and collector capability",
        ));
    }
    Ok(())
}
pub(super) fn baseline(journal: &Journal, data: &Value) -> Result<(), FailureDetail> {
    capability(data)?;
    for binding in &journal.manifest.bindings {
        capture_binding(data, binding)?;
        for name in [
            &binding.evidence.state_before,
            &binding.evidence.state_after,
        ] {
            state(data, name)?;
        }
        capture_artifact(data, binding)?;
    }
    Ok(())
}
fn state<'a>(data: &'a Value, name: &str) -> Result<&'a Value, FailureDetail> {
    let value = &data["state_records"][name];
    if !value.is_object() || serde_json::to_vec(value).map_or(true, |v| v.len() > 64 * 1024) {
        return Err(fail(
            "asset_trial_evidence_missing",
            name,
            "bounded record_state evidence is required",
        ));
    }
    Ok(value)
}
fn capture_binding<'a>(data: &'a Value, binding: &Binding) -> Result<&'a Value, FailureDetail> {
    let items = data["asset_trial"]["captures"][binding.evidence.name()]["bindings"]
        .as_array()
        .ok_or_else(|| {
            fail(
                "asset_binding_unconfirmed",
                &binding.id,
                "capture binding facts missing",
            )
        })?;
    let found: Vec<_> = items
        .iter()
        .filter(|item| item["id"] == binding.id)
        .collect();
    if found.len() != 1 || found[0].get("error").is_some() {
        return Err(fail(
            "asset_binding_unconfirmed",
            &binding.id,
            "runtime target must resolve uniquely at the selected capture",
        ));
    }
    Ok(found[0])
}
pub(super) fn capture_artifact(data: &Value, binding: &Binding) -> Result<PathBuf, FailureDetail> {
    let name = binding.evidence.name();
    let path = if binding.evidence.snapshot.is_some() {
        data["snapshot_artifacts"][name]["screenshot"].as_str()
    } else {
        data["audio_artifacts"][name]["wav"].as_str()
    };
    let path = path
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .ok_or_else(|| {
            fail(
                "asset_trial_evidence_missing",
                name,
                "selected capture artifact is absent",
            )
        })?;
    Ok(path)
}
pub(super) fn candidate(journal: &Journal, data: &Value) -> Result<(), FailureDetail> {
    baseline(journal, data)?;
    let before = journal.baseline.as_ref().ok_or_else(|| {
        fail(
            "asset_trial_evidence_missing",
            &journal.id,
            "baseline is absent",
        )
    })?;
    let probe = journal.import_probe.as_ref().ok_or_else(|| {
        fail(
            "asset_trial_evidence_missing",
            &journal.id,
            "import probe is absent",
        )
    })?;
    if data["asset_trial"]["backend"] != before["asset_trial"]["backend"]
        || data["asset_trial"]["backend"] != probe["backend"]
    {
        return Err(fail(
            "asset_trial_noncomparable",
            &journal.id,
            "baseline, import probe, and candidate rendering/audio backends differ",
        ));
    }
    for binding in &journal.manifest.bindings {
        for name in [
            &binding.evidence.state_before,
            &binding.evidence.state_after,
        ] {
            // record_state includes observation frames; compare authored values only.
            let a = state(before, name)?;
            let b = state(data, name)?;
            if checkpoint_values(a) != checkpoint_values(b) {
                let mut f = fail(
                    "asset_trial_noncomparable",
                    name,
                    "before/after gameplay state differs across trial phases",
                );
                f.expected = Some(checkpoint_values(a).clone());
                f.actual = Some(checkpoint_values(b).clone());
                return Err(f);
            }
        }
        let actual = capture_binding(data, binding)?;
        let expected = probe["data"]["resources"]
            .as_array()
            .and_then(|a| a.iter().find(|r| r["path"] == binding.destination))
            .ok_or_else(|| {
                fail(
                    "asset_fingerprint_unavailable",
                    &binding.destination,
                    "fresh imported resource fingerprint is missing",
                )
            })?;
        let resource = &actual["resource"];
        if resource["fingerprint"]["sha256"]
            .as_str()
            .is_none_or(str::is_empty)
            || expected["fingerprint"]["sha256"]
                .as_str()
                .is_none_or(str::is_empty)
        {
            return Err(fail(
                "asset_fingerprint_unavailable",
                &binding.id,
                "runtime or imported resource content readback is missing",
            ));
        }
        if actual["binding_changed"].as_bool() != Some(false)
            || actual["resource_after"]["fingerprint"] != resource["fingerprint"]
            || actual["resource_after"]["path"] != resource["path"]
            || resource["path"] != binding.destination
            || resource["fingerprint"].is_null()
            || resource["fingerprint"] != expected["fingerprint"]
            || resource["source_sha256"] != expected["source_sha256"]
            || resource["class"] != expected["class"]
            || resource["uid"] != expected["uid"]
        {
            return Err(fail(
                "asset_binding_unconfirmed",
                &binding.id,
                "runtime binding path/content does not match the fresh import probe",
            ));
        }
        if binding.evidence.snapshot.is_some() {
            if actual["visible_pixel_count"].as_u64().unwrap_or(0) == 0
                || actual["rendered_frames"].as_u64().unwrap_or(0) == 0
                || actual["observation_overflow"].as_bool() == Some(true)
            {
                return Err(fail(
                    "asset_binding_unconfirmed",
                    &binding.id,
                    "candidate texture has no visible target pixels at capture",
                ));
            }
        } else {
            if data["asset_trial"]["backend"]["audio_driver"]
                .as_str()
                .is_none_or(|s| s.is_empty() || s.eq_ignore_ascii_case("Dummy"))
                || actual["playback_overlap"].as_bool() != Some(true)
                || actual["binding_changed"].as_bool() == Some(true)
                || actual["observation_overflow"].as_bool() == Some(true)
                || actual["observed_frames"].as_u64().unwrap_or(0) == 0
                || actual["bus_changed"].as_bool() == Some(true)
                || actual["bus_matches"].as_bool() != Some(true)
            {
                return Err(fail(
                    "asset_binding_unconfirmed",
                    &binding.id,
                    "audio requires actual driver, stable stream, and observed playback on the captured bus",
                ));
            }
            let wav = capture_artifact(data, binding)?;
            let analysis = crate::audio::analyze_wav_file(&wav).map_err(|e| {
                fail(
                    "asset_trial_audio_invalid",
                    &wav.display().to_string(),
                    &e.to_string(),
                )
            })?;
            let requested = journal.scenario["steps"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|s| s["audio_capture"]["name"] == binding.evidence.name())
                .and_then(|s| s["audio_capture"]["duration_seconds"].as_f64())
                .ok_or_else(|| {
                    fail(
                        "asset_trial_audio_invalid",
                        &binding.id,
                        "capture duration metadata missing",
                    )
                })?;
            if analysis["near_silence"].as_bool() != Some(false)
                || (analysis["duration_seconds"].as_f64().unwrap_or(0.0) - requested).abs()
                    > 0.1_f64.max(requested * 0.1)
            {
                return Err(fail(
                    "asset_trial_audio_invalid",
                    &binding.id,
                    "candidate audio is near silent or capture duration differs by more than max(100ms, 10%)",
                ));
            }
        }
    }
    Ok(())
}
fn checkpoint_values(value: &Value) -> &Value {
    value
        .get("value")
        .or_else(|| value.get("values"))
        .or_else(|| value.get("state"))
        .unwrap_or(value)
}

pub(super) fn scene_gate(
    original: &str,
    candidate: &str,
    manifest: &Manifest,
    writer: &Value,
) -> Result<(), FailureDetail> {
    let bad = || {
        fail(
            "asset_trial_unsupported_roundtrip",
            &manifest.scene,
            "saved scene changed unsupported/unrequested semantics; original source is preserved",
        )
    };
    if writer.pointer("/data/semantic_gate/ok") != Some(&Value::Bool(true)) {
        return Err(bad());
    }
    let a = godot_qa_scene::parse_scene(original);
    let b = godot_qa_scene::parse_scene(candidate);
    if a.has_errors()
        || b.has_errors()
        || a.nodes.len() != b.nodes.len()
        || a.connections != b.connections
        || a.editable != b.editable
        || header_uid(original) != header_uid(candidate)
    {
        return Err(bad());
    }
    if a.root().is_none_or(|root| root.is_instance()) {
        return Err(bad());
    }
    for (old, new) in a.nodes.iter().zip(&b.nodes) {
        if old.name != new.name
            || old.parent != new.parent
            || (old.type_name != new.type_name
                && !(old.instance_scene.is_some() && new.type_name.is_none()))
            || old.instance_scene != new.instance_scene
            || old.instance_unresolved != new.instance_unresolved
            || old.index != new.index
            || old.groups != new.groups
        {
            return Err(bad());
        }
        let path = old.relative_path();
        for (property, value) in &old.properties {
            if manifest
                .bindings
                .iter()
                .any(|binding| binding.edit_path == path && binding.property == *property)
            {
                continue;
            }
            if new.property(property) != Some(value) {
                // The engine gate resolves embedded resources and default elision;
                // the parser deliberately cannot normalize these independently.
                if !value.is_unsupported() && new.property(property).is_some() {
                    return Err(bad());
                }
            }
        }
    }
    for binding in &manifest.bindings {
        let node = b
            .nodes
            .iter()
            .find(|n| n.relative_path() == binding.edit_path)
            .ok_or_else(bad)?;
        if node
            .property(&binding.property)
            .and_then(godot_qa_scene::GodotValue::as_ext_resource)
            != Some(binding.destination.as_str())
        {
            return Err(bad());
        }
    }
    Ok(())
}
pub(super) fn header_uid(text: &str) -> String {
    text.lines()
        .find(|l| l.trim_start().starts_with("[gd_scene "))
        .and_then(|line| line.split("uid=\"").nth(1))
        .and_then(|s| s.split('"').next())
        .unwrap_or_default()
        .to_owned()
}
