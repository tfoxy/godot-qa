use super::*;
use std::collections::BTreeMap;

fn root() -> PathBuf {
    let root = std::env::temp_dir().join(format!("godot-qa-trial-test-{}", random_id().unwrap()));
    fs::create_dir_all(&root).unwrap();
    // Production journals store the canonical path, including the Windows prefix.
    fs::canonicalize(root).unwrap()
}
fn journal(root: &Path) -> Journal {
    let id = random_id().unwrap();
    let dir = root.join(".qa/asset-trials").join(&id);
    fs::create_dir_all(&dir).unwrap();
    write_new(&dir.join("original.tscn"), b"original").unwrap();
    Journal {
        format: "godot-qa.asset-trial-journal.v1".into(),
        id,
        project: root.into(),
        manifest: Manifest {
            format: "godot-qa.asset-trial.v1".into(),
            scene: "res://scene.tscn".into(),
            validation: manifest::Validation {
                scenario: Some("test".into()),
                scenario_file: None,
            },
            bindings: vec![],
        },
        scenario: json!({}),
        execution_config: json!({}),
        candidate_hashes: vec![],
        source_paths: vec![],
        state: "prepared".into(),
        phase: "prepared".into(),
        intent: None,
        original_hash: godot_qa_core::hash::sha256_hex(b"original"),
        output_hash: Some(godot_qa_core::hash::sha256_hex(b"candidate")),
        scene_uid: Some("".into()),
        owned: BTreeMap::new(),
        media_captures: BTreeMap::new(),
        frozen: BTreeMap::new(),
        evidence_root: root.join("evidence"),
        evidence_hashes: BTreeMap::new(),
        baseline: None,
        candidate: None,
        import_probe: None,
        failures: vec![],
        warnings: vec![],
    }
}
#[test]
fn no_clobber_publication_preserves_every_race_winner() {
    for boundary in [
        "capture_intent",
        "capture_moved",
        "captured",
        "publish_intent",
        "published_link",
        "published",
    ] {
        let root = root();
        let mut j = journal(&root);
        let target = root.join("scene.tscn");
        let output = j.dir().join("output.tscn");
        write_new(&target, b"original").unwrap();
        write_new(&output, b"candidate").unwrap();
        let hash = j.original_hash.clone();
        let mut hit = false;
        let r = replace_no_clobber(&mut j, &target, &output, &hash, "apply", &mut |phase| {
            if phase == boundary {
                hit = true;
                return Err(fail("injected_crash", phase, "injected"));
            }
            Ok(())
        });
        assert!(r.is_err());
        assert!(hit);
        let captured = j.dir().join("apply-captured.tscn");
        assert!(target.exists() || captured.exists());
        if target.exists() {
            let b = fs::read(&target).unwrap();
            assert!(b == b"original" || b == b"candidate");
        }
        assert_eq!(
            fs::read(j.dir().join("original.tscn")).unwrap(),
            b"original"
        );
        restore_scene_files(&mut j).unwrap();
        assert_eq!(
            fs::read(&target).unwrap(),
            b"original",
            "recovery at {boundary}"
        );
        restore_scene_files(&mut j).unwrap();
        assert_eq!(
            fs::read(&target).unwrap(),
            b"original",
            "idempotent recovery at {boundary}"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
#[test]
fn raced_edit_is_reinstalled_without_overwrite() {
    let root = root();
    let mut j = journal(&root);
    let target = root.join("scene.tscn");
    let output = j.dir().join("output.tscn");
    write_new(&target, b"original").unwrap();
    write_new(&output, b"candidate").unwrap();
    let hash = j.original_hash.clone();
    let error = replace_no_clobber(&mut j, &target, &output, &hash, "apply", &mut |phase| {
        if phase == "capture_intent" {
            fs::write(&target, b"user edit").unwrap();
        }
        Ok(())
    })
    .unwrap_err();
    assert_eq!(error.kind, "asset_trial_conflict");
    assert_eq!(fs::read(&target).unwrap(), b"user edit");
    assert_eq!(
        fs::read(j.dir().join("apply-captured.tscn")).unwrap(),
        b"user edit"
    );
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn competing_scene_creation_preserves_both_versions() {
    let root = root();
    let mut j = journal(&root);
    let target = root.join("scene.tscn");
    let output = j.dir().join("output.tscn");
    write_new(&target, b"original").unwrap();
    write_new(&output, b"candidate").unwrap();
    let hash = j.original_hash.clone();
    assert!(
        replace_no_clobber(&mut j, &target, &output, &hash, "apply", &mut |phase| {
            if phase == "publish_intent" {
                write_new(&target, b"competing creation")?;
            }
            Ok(())
        })
        .is_err()
    );
    assert_eq!(fs::read(&target).unwrap(), b"competing creation");
    assert_eq!(
        fs::read(j.dir().join("apply-captured.tscn")).unwrap(),
        b"original"
    );
    assert_eq!(fs::read(output).unwrap(), b"candidate");
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn paths_reject_traversal_and_links() {
    let root = root();
    for bad in [
        "../x",
        "res://../x",
        "res:///tmp/x",
        "res://a/../x",
        "res://a\\b",
        "res://",
    ] {
        assert!(manifest::project_path(&root, bad).is_err(), "{bad}");
    }
    assert_eq!(
        manifest::project_path(&root, "res://assets/new.png").unwrap(),
        root.join("assets/new.png")
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(std::env::temp_dir(), root.join("link")).unwrap();
        assert!(manifest::project_path(&root, "res://link/x").is_err());
    }
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn rejects_malformed_and_mismatched_media() {
    assert!(manifest::media(b"fake", "png").is_err());
    assert!(manifest::media(b"RIFF0000WAVE", "wav").is_err());
    let mut huge = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    huge.extend_from_slice(&4097u32.to_be_bytes());
    huge.extend_from_slice(&1u32.to_be_bytes());
    assert!(manifest::media(&huge, "png").is_err());
}
#[test]
fn trial_evidence_pin_blocks_explicit_and_automatic_retention() {
    let root = root();
    write_new(&root.join(".asset-trial-pin"), b"pinned").unwrap();
    fs::create_dir_all(root.join("before/fixture/100Z")).unwrap();
    fs::create_dir_all(root.join("before/fixture/200Z")).unwrap();
    assert!(
        crate::retention::prune_scenario_runs(&root.join("before"), "fixture", 1)
            .removed
            .is_empty()
    );
    assert!(root.join("before/fixture/100Z").exists());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn pinned_backups_are_independent_of_scene_in_place_edits() {
    let root = root();
    let j = journal(&root);
    write_new(&root.join("scene.tscn"), b"original").unwrap();
    fs::write(root.join("scene.tscn"), b"changed").unwrap();
    assert_eq!(
        fs::read(j.dir().join("original.tscn")).unwrap(),
        b"original"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn preload_does_not_hide_dynamic_load_uncertainty() {
    assert!(has_dynamic_load(
        "var a=preload(\"res://a.png\"); var b=load(path)"
    ));
    assert!(!has_dynamic_load("load(\"res://a.png\")"));
    assert!(has_dynamic_load("load(\"res://\" + name)"));
}

fn evidence_fixture(root: &Path) -> (Journal, Value) {
    let mut j = journal(root);
    let binding = Binding {
        id: "coin".into(),
        source: "source.png".into(),
        destination: "res://coin.png".into(),
        edit_path: "Coin".into(),
        runtime_target: "qa:coin".into(),
        property: "texture".into(),
        evidence: manifest::Evidence {
            snapshot: Some("capture".into()),
            audio: None,
            state_before: "before".into(),
            state_after: "after".into(),
        },
    };
    j.manifest.bindings.push(binding);
    let screenshot = root.join("capture.png");
    write_new(&screenshot, b"capture proof path").unwrap();
    let backend = json!({"display_server":"X11","rendering_method":"gl_compatibility","rendering_driver":"opengl3","audio_driver":"PulseAudio"});
    let resource = json!({"path":"res://coin.png","source_sha256":"source","fingerprint":{"kind":"rgba8","width":8,"height":8,"sha256":"loaded"}});
    let data = json!({"state_records":{"before":{"value":{"coins":0}},"after":{"value":{"coins":0}}},"snapshot_artifacts":{"capture":{"screenshot":screenshot}},"asset_trial":{"version":1,"capabilities":["asset_trial_v1"],"backend":backend,"captures":{"capture":{"kind":"snapshot","bindings":[{"id":"coin","resource":resource,"resource_after":resource,"binding_changed":false,"rendered_frames":2,"visible_pixel_count":8}]}}}});
    j.baseline = Some(data.clone());
    j.import_probe = Some(json!({"backend":backend,"data":{"resources":[resource]}}));
    (j, data)
}
#[test]
fn candidate_proof_rejects_stale_invisible_unrendered_and_changed_bindings() {
    let root = root();
    let (j, good) = evidence_fixture(&root);
    assert!(evidence::candidate(&j, &good).is_ok());
    for (pointer, value) in [
        (
            "/asset_trial/captures/capture/bindings/0/resource/path",
            json!("res://wrong.png"),
        ),
        (
            "/asset_trial/captures/capture/bindings/0/visible_pixel_count",
            json!(0),
        ),
        (
            "/asset_trial/captures/capture/bindings/0/rendered_frames",
            json!(0),
        ),
        (
            "/asset_trial/captures/capture/bindings/0/binding_changed",
            json!(true),
        ),
        (
            "/asset_trial/captures/capture/bindings/0/resource_after/fingerprint/sha256",
            json!("rebound"),
        ),
    ] {
        let mut bad = good.clone();
        *bad.pointer_mut(pointer).unwrap() = value;
        assert_eq!(
            evidence::candidate(&j, &bad).unwrap_err().kind,
            "asset_binding_unconfirmed",
            "{pointer}"
        );
    }
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn candidate_requires_comparable_checkpoints_backend_and_capability() {
    let root = root();
    let (j, good) = evidence_fixture(&root);
    for pointer in [
        "/state_records/before/value/coins",
        "/state_records/after/value/coins",
    ] {
        let mut bad = good.clone();
        *bad.pointer_mut(pointer).unwrap() = json!(1);
        assert_eq!(
            evidence::candidate(&j, &bad).unwrap_err().kind,
            "asset_trial_noncomparable"
        );
    }
    let mut bad = good.clone();
    bad["asset_trial"]["backend"]["display_server"] = json!("headless");
    assert_eq!(
        evidence::candidate(&j, &bad).unwrap_err().kind,
        "asset_trial_noncomparable"
    );
    bad["asset_trial"]["capabilities"] = json!([]);
    assert_eq!(
        evidence::candidate(&j, &bad).unwrap_err().kind,
        "addon_capability_missing"
    );
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn null_baseline_slot_is_factual_and_not_a_candidate_failure() {
    let root = root();
    let (mut j, good) = evidence_fixture(&root);
    j.baseline.as_mut().unwrap()["asset_trial"]["captures"]["capture"]["bindings"][0]["resource"] =
        json!({"null":true});
    assert!(evidence::baseline(&j, j.baseline.as_ref().unwrap()).is_ok());
    assert!(evidence::candidate(&j, &good).is_ok());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reverse_publication_crashes_recover_byte_exactly_at_every_boundary() {
    for boundary in [
        "capture_intent",
        "capture_moved",
        "captured",
        "publish_intent",
        "published_link",
        "published",
    ] {
        let root = root();
        let mut j = journal(&root);
        let target = root.join("scene.tscn");
        write_new(&target, b"candidate").unwrap();
        write_new(&j.dir().join("apply-captured.tscn"), b"original").unwrap();
        let replacement = j.dir().join("restore-output.tscn");
        write_new(&replacement, b"original").unwrap();
        let hash = j.output_hash.clone().unwrap();
        assert!(
            replace_no_clobber(
                &mut j,
                &target,
                &replacement,
                &hash,
                "restore",
                &mut |phase| {
                    if phase == boundary {
                        return Err(fail("injected_crash", phase, "injected"));
                    }
                    Ok(())
                }
            )
            .is_err()
        );
        restore_scene_files(&mut j).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"original", "{boundary}");
        restore_scene_files(&mut j).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"original");
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn preflight_rejects_disabled_captures_baselines_and_uncomparable_scenarios() {
    let root = root();
    let (j, _) = evidence_fixture(&root);
    let scenario = json!({"viewport":{"width":960,"height":540},"determinism":{"seed":1,"fixed_fps":60,"locale":"en"},"project_args":[],"steps":[{"record_state":{"name":"before"}},{"snapshot":"capture"},{"record_state":{"name":"after"}}],"assert":[{"state_matches":{"matches":[]}}]});
    assert!(manifest::validate_evidence(&j.manifest, &scenario).is_ok());
    let mut bad = scenario.clone();
    bad["snapshots"] = json!({"save_screenshot":false});
    assert!(manifest::validate_evidence(&j.manifest, &bad).is_err());
    bad = scenario.clone();
    bad["assert"]
        .as_array_mut()
        .unwrap()
        .push(json!({"screenshot_match":{"snapshot":"capture","baseline":"qa/baseline.png"}}));
    assert!(manifest::validate_evidence(&j.manifest, &bad).is_err());
    bad = scenario.clone();
    bad["assert"] = json!([{"no_errors":true}]);
    assert!(manifest::validate_evidence(&j.manifest, &bad).is_err());
    bad = scenario.clone();
    bad["viewport"] = json!({});
    assert!(manifest::validate_evidence(&j.manifest, &bad).is_err());
    bad = scenario;
    bad.as_object_mut().unwrap().remove("project_args");
    assert!(manifest::validate_evidence(&j.manifest, &bad).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn frozen_sources_include_external_candidates_and_new_project_dependencies() {
    let root = root();
    let mut j = journal(&root);
    let external = root.with_extension("png");
    write_new(&external, b"source").unwrap();
    j.source_paths.push(external.clone());
    j.candidate_hashes.push(file_hash(&external).unwrap());
    j.frozen = snapshot_tree(&root, &[]).unwrap();
    assert!(verify_frozen(&j).is_ok());
    fs::write(&external, b"changed").unwrap();
    assert_eq!(verify_frozen(&j).unwrap_err().kind, "asset_trial_conflict");
    fs::write(&external, b"source").unwrap();
    write_new(&root.join("new.gd"), b"extends Node").unwrap();
    assert_eq!(
        verify_frozen(&j).unwrap_err().kind,
        "asset_trial_noncomparable"
    );
    fs::remove_file(external).unwrap();
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn terminal_retries_clear_only_their_own_interrupted_reservation() {
    let root = root();
    let mut j = journal(&root);
    j.save().unwrap();
    j.state = "kept".into();
    j.save().unwrap();
    clear_terminal_reservation(&j).unwrap();
    assert!(!root.join(".qa/asset-trials/active.json").exists());
    atomic_json(
        &root.join(".qa/asset-trials/active.json"),
        &json!({"trial_id":"other","state":"prepared"}),
    )
    .unwrap();
    clear_terminal_reservation(&j).unwrap();
    assert_eq!(
        read_json(&root.join(".qa/asset-trials/active.json")).unwrap()["trial_id"],
        "other"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn candidate_result_is_incomplete_until_technical_evaluation_finishes() {
    let root = root();
    let mut j = journal(&root);
    j.phase = "applied".into();
    j.candidate = Some(json!({}));
    assert_eq!(
        result("asset status", &j).data["technical_status"],
        "incomplete"
    );
    j.save().unwrap();
    assert_eq!(
        read_json(&j.dir().join("review.json")).unwrap()["technical_status"],
        "incomplete"
    );
    j.phase = "evaluated".into();
    assert_eq!(result("asset status", &j).data["technical_status"], "pass");
    j.state = "kept".into();
    assert_eq!(result("asset status", &j).data["technical_status"], "pass");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unjournaled_scene_deletion_is_never_recreated_from_stale_capture() {
    let root = root();
    let mut j = journal(&root);
    j.phase = "evaluated".into();
    j.state = "evaluated".into();
    write_new(&j.dir().join("apply-captured.tscn"), b"original").unwrap();
    assert_eq!(
        restore_scene_files(&mut j).unwrap_err().kind,
        "asset_trial_conflict"
    );
    assert!(!root.join("scene.tscn").exists());
    j.intent = Some("apply_published".into());
    assert!(restore_scene_files(&mut j).is_err());
    assert!(!root.join("scene.tscn").exists());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn identical_foreign_stage_race_is_not_owned() {
    let root = root();
    let (mut j, _) = evidence_fixture(&root);
    let binding = j.manifest.bindings[0].clone();
    j.candidate_hashes
        .push(godot_qa_core::hash::sha256_hex(b"candidate"));
    write_new(&j.dir().join("candidate-0.bin"), b"candidate").unwrap();
    let error = stage_binding(&mut j, &binding, 0, &mut |path| {
        write_new(path, b"candidate")
    })
    .unwrap_err();
    assert_eq!(error.details["io_kind"], "AlreadyExists");
    assert!(!j.owned.contains_key(&binding.destination));
    assert_eq!(fs::read(root.join("coin.png")).unwrap(), b"candidate");
    assert!(j.intent.is_none());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn loader_uncertainty_covers_whitespace_threaded_and_generic_forms() {
    for source in [
        "load (path)",
        "ResourceLoader.load_threaded_request(path)",
        "GD.Load<Texture2D>(path)",
        "GD.Load <Texture2D> (path)",
        "preload(\"res://known\"); load \n (variable)",
    ] {
        assert!(has_dynamic_load(source), "{source}");
    }
}
fn test_config(root: &Path) -> ProjectConfig {
    write_new(&root.join("project.godot"), b"config_version=5\n").unwrap();
    godot_qa_core::config::load_project_config(Some(root), None, None, false, None)
}
#[test]
fn journal_integrity_rejects_redirect_to_identical_unrelated_scene() {
    let root = root();
    let config = test_config(&root);
    let j = journal(&root);
    write_new(&root.join("unrelated.tscn"), b"candidate").unwrap();
    j.save().unwrap();
    assert!(load(&config, &j.id).is_ok());
    let path = j.dir().join("journal.json");
    let mut raw = read_json(&path).unwrap();
    raw["manifest"]["scene"] = json!("res://unrelated.tscn");
    atomic_json(&path, &raw).unwrap();
    assert_eq!(
        load(&config, &j.id).unwrap_err().kind,
        "asset_trial_journal_invalid"
    );
    assert!(!run_asset_discard(&config, &j.id).ok);
    assert_eq!(fs::read(root.join("unrelated.tscn")).unwrap(), b"candidate");
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn prepared_journal_without_reservation_can_be_discarded_after_setup_crash() {
    let root = root();
    let config = test_config(&root);
    let mut j = journal(&root);
    j.output_hash = None;
    j.save_journal().unwrap();
    assert!(!root.join(".qa/asset-trials/active.json").exists());
    assert!(load(&config, &j.id).is_ok());
    let result = run_asset_discard(&config, &j.id);
    assert!(result.ok, "{:?}", result.failures);
    assert_eq!(result.data["state"], "discarded");
    assert!(!root.join(".qa/asset-trials/active.json").exists());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn owned_media_custody_recovers_every_interruption_without_touching_foreign_bytes() {
    for boundary in [
        "capture_intent",
        "capture_moved",
        "captured",
        "delete_intent",
        "deleted",
    ] {
        let root = root();
        let mut j = journal(&root);
        let resource = "res://media.png";
        let path = root.join("media.png");
        let hash = godot_qa_core::hash::sha256_hex(b"ours");
        j.owned.insert(resource.into(), hash.clone());
        write_new(&path, b"ours").unwrap();
        assert!(
            remove_owned_media(&mut j, resource, &hash, &mut |phase| {
                if phase == boundary {
                    return Err(fail("injected_crash", phase, "injected"));
                }
                Ok(())
            })
            .is_err()
        );
        remove_owned_media(&mut j, resource, &hash, &mut |_| Ok(())).unwrap();
        assert!(!path.exists(), "{boundary}");
        remove_owned_media(&mut j, resource, &hash, &mut |_| Ok(())).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
#[test]
fn media_replacement_before_capture_is_preserved_byte_for_byte() {
    let root = root();
    let mut j = journal(&root);
    let path = root.join("media.png");
    let hash = godot_qa_core::hash::sha256_hex(b"ours");
    j.owned.insert("res://media.png".into(), hash.clone());
    write_new(&path, b"ours").unwrap();
    assert_eq!(
        remove_owned_media(&mut j, "res://media.png", &hash, &mut |phase| {
            if phase == "capture_intent" {
                fs::write(&path, b"foreign").unwrap();
            }
            Ok(())
        })
        .unwrap_err()
        .kind,
        "asset_trial_conflict"
    );
    assert_eq!(fs::read(&path).unwrap(), b"foreign");
    let captured = j.dir().join(&j.media_captures["res://media.png"]);
    assert_eq!(fs::read(captured).unwrap(), b"foreign");
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn media_replacement_after_capture_is_never_deleted_on_retry() {
    let root = root();
    let mut j = journal(&root);
    let path = root.join("media.png");
    let hash = godot_qa_core::hash::sha256_hex(b"ours");
    j.owned.insert("res://media.png".into(), hash.clone());
    write_new(&path, b"ours").unwrap();
    remove_owned_media(&mut j, "res://media.png", &hash, &mut |phase| {
        if phase == "capture_moved" {
            write_new(&path, b"foreign")?;
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"foreign");
    remove_owned_media(&mut j, "res://media.png", &hash, &mut |_| Ok(())).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"foreign");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_extension_presence_retains_unproven_media() {
    let root = root();
    let config = test_config(&root);
    let (j, _) = evidence_fixture(&root);
    write_new(&root.join("custom.gdextension"), b"[configuration]\n").unwrap();
    assert!(
        source_references(&config, &j)
            .unwrap()
            .contains(&"res://coin.png".to_string())
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_runtime_fingerprint_or_capture_file_never_counts_as_proof() {
    let root = root();
    let (j, good) = evidence_fixture(&root);
    let mut bad = good.clone();
    bad["asset_trial"]["captures"]["capture"]["bindings"][0]["resource"]
        .as_object_mut()
        .unwrap()
        .remove("fingerprint");
    assert_eq!(
        evidence::candidate(&j, &bad).unwrap_err().kind,
        "asset_fingerprint_unavailable"
    );
    fs::remove_file(root.join("capture.png")).unwrap();
    assert_eq!(
        evidence::candidate(&j, &good).unwrap_err().kind,
        "asset_trial_evidence_missing"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unresolved_stage_intent_never_adopts_identical_foreign_bytes_after_crash() {
    let root = root();
    let (mut j, _) = evidence_fixture(&root);
    let binding = j.manifest.bindings[0].clone();
    j.candidate_hashes
        .push(godot_qa_core::hash::sha256_hex(b"candidate"));
    write_new(&j.dir().join("candidate-0.bin"), b"candidate").unwrap();
    assert!(
        stage_binding(&mut j, &binding, 0, &mut |path| {
            write_new(path, b"candidate")?;
            Err(fail(
                "injected_crash",
                "stage",
                "crash before ownership receipt",
            ))
        })
        .is_err()
    );
    assert!(j.owned.contains_key(&binding.destination));
    assert!(j.intent.as_deref().unwrap().starts_with("stage:"));
    retain_unconfirmed_stage(&mut j).unwrap();
    assert!(!j.owned.contains_key(&binding.destination));
    assert_eq!(fs::read(root.join("coin.png")).unwrap(), b"candidate");
    assert!(
        read_json(&j.dir().join("journal.json")).unwrap()["owned"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    assert!(
        j.warnings
            .iter()
            .any(|w| w.contains("Ownership was not confirmed"))
    );
    retain_unconfirmed_stage(&mut j).unwrap();
    assert_eq!(fs::read(root.join("coin.png")).unwrap(), b"candidate");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn journal_checksum_survives_precise_audio_floats_across_repeated_loads() {
    let root = root();
    let config = test_config(&root);
    let mut j = journal(&root);
    // Real PCM capture duration: without serde_json/float_roundtrip the parser
    // rounds this one ULP lower, invalidating a journal written moments earlier.
    j.baseline = Some(
        json!({"audio_analysis":{"sfx":{"duration_seconds":0.9984580498866213,"peak_by_window":[{"end_seconds":0.9984580498866213}]}}}),
    );
    j.candidate = Some(
        json!({"metrics":[0.8455124082255701,1.2345678901234567,1e-100,-1.2345678901234567e100]}),
    );
    let expected = json!(j);
    for _ in 0..3 {
        j.save_journal().unwrap();
        j = load(&config, &j.id).unwrap();
        assert_eq!(json!(j), expected);
    }
    fs::remove_dir_all(root).unwrap();
}
