extends RefCounted

const QaSnapshot = preload("res://addons/godot_qa/qa_snapshot.gd")

var _owner: WeakRef
var _runtime: RefCounted:
	get:
		return _owner.get_ref()._runtime
var _root_window: Window:
	get:
		return _owner.get_ref()._root_window


func _init(owner: RefCounted) -> void:
	_owner = weakref(owner)


func _assert_visible_pixels(params: Dictionary) -> Dictionary:
	var validation := _validate_visual_pixel_params(params, "assert.visible_pixels", true)
	if not bool(validation.get("ok", false)):
		return validation
	var target := str(validation.get("target", ""))
	var probe := await _visual_probe(target, params, "assert.visible_pixels")
	if not bool(probe.get("ok", false)):
		return probe
	var ratio := float(probe.get("visible_pixel_ratio", 0.0))
	var count := int(probe.get("visible_pixel_count", 0))
	var ratio_passes := true
	if bool(validation.get("has_min_ratio", false)):
		ratio_passes = ratio >= float(validation.get("min_ratio", 0.0))
	var count_passes := true
	if bool(validation.get("has_min_count", false)):
		count_passes = count >= int(validation.get("min_count", 0))
	var threshold: Dictionary = validation.get("threshold", {})
	if ratio_passes and count_passes:
		var payload: Dictionary = probe.duplicate(true)
		payload["threshold"] = threshold
		return _visual_required_artifact_result(payload)
	var fail_payload: Dictionary = probe.duplicate(true)
	fail_payload["threshold"] = threshold
	fail_payload["reason"] = "visible_pixel_count_below_threshold" if not count_passes else "visible_pixel_ratio_below_threshold"
	return _owner.get_ref()._error("visible_pixels_mismatch", "target rendered contribution was below threshold", fail_payload)


func _assert_not_occluded(params: Dictionary) -> Dictionary:
	var validation := _validate_visual_pixel_params(params, "assert.not_occluded", false)
	if not bool(validation.get("ok", false)):
		return validation
	var occluder := str(params.get("occluder", "")).strip_edges()
	if params.has("occluder") and occluder == "":
		return _owner.get_ref()._error("invalid_assertion_args", "not_occluded occluder must be non-empty when provided", {"command": "assert.not_occluded", "target": validation.get("target", ""), "snapshot": validation.get("snapshot", ""), "field": "occluder", "actual": params.get("occluder", null)})
	var probe := await _visual_probe(str(validation.get("target", "")), params, "assert.not_occluded")
	if not bool(probe.get("ok", false)):
		return probe
	var threshold: Dictionary = validation.get("threshold", {})
	if not bool(validation.get("has_min_ratio", false)):
		threshold["min_ratio"] = 0.0001
	var ratio_threshold := float(threshold.get("min_ratio", 0.0001))
	var ratio := float(probe.get("visible_pixel_ratio", 0.0))
	var count := int(probe.get("visible_pixel_count", 0))
	var ratio_passes := ratio >= ratio_threshold
	var count_passes := true
	if bool(validation.get("has_min_count", false)):
		count_passes = count >= int(validation.get("min_count", 0))
	if ratio_passes and count_passes:
		var pass_payload: Dictionary = probe.duplicate(true)
		pass_payload["threshold"] = threshold
		pass_payload["occluder"] = occluder
		return _visual_required_artifact_result(pass_payload)
	var fail_payload: Dictionary = probe.duplicate(true)
	fail_payload["threshold"] = threshold
	fail_payload["reason"] = "target_visible_pixel_count_below_threshold" if not count_passes else "target_has_no_or_low_visible_contribution"
	fail_payload["occluder"] = occluder
	return _owner.get_ref()._error("not_occluded_mismatch", "target appears fully occluded", fail_payload)


func _assert_renders_above(params: Dictionary) -> Dictionary:
	return await _assert_render_order(params, true)


func _assert_renders_below(params: Dictionary) -> Dictionary:
	return await _assert_render_order(params, false)


func _assert_render_order(params: Dictionary, expect_above: bool) -> Dictionary:
	var command_name := "assert.renders_above" if expect_above else "assert.renders_below"
	var validation := _validate_visual_order_params(params, command_name)
	if not bool(validation.get("ok", false)):
		return validation
	var a := str(validation.get("target", ""))
	var b := str(validation.get("other", ""))
	var snapshot := _visual_snapshot(params)
	var left = _owner.get_ref()._require_target(a)
	if not bool(left.get("ok", false)):
		return _visual_error_with_context(left, a, params, command_name)
	var right = _owner.get_ref()._require_target(b)
	if not bool(right.get("ok", false)):
		return _visual_error_with_context(right, b, params, command_name)
	var node_a: Node = left.get("node")
	var node_b: Node = right.get("node")
	var base_payload := {
		"command": command_name,
		"target": a,
		"other": b,
		"snapshot": snapshot,
		"a": a,
		"b": b,
		"a_resolved": left.get("resolved", ""),
		"b_resolved": right.get("resolved", ""),
		"expected": "target_above_other" if expect_above else "target_below_other",
	}
	var requested_evidence := str(params.get("evidence", "auto")).strip_edges()
	if requested_evidence == "":
		requested_evidence = "auto"
	if not ["auto", "canvas_order", "pixel_overlap"].has(requested_evidence):
		var evidence_payload := base_payload.duplicate(true)
		evidence_payload["actual"] = requested_evidence
		evidence_payload["expected"] = ["auto", "canvas_order", "pixel_overlap"]
		return _owner.get_ref()._error("invalid_assertion_args", "render-order evidence must be auto, canvas_order, or pixel_overlap", evidence_payload)
	var require_overlap := bool(params.get("require_overlap", false)) or requested_evidence == "pixel_overlap"
	if not (node_a is CanvasItem and node_b is CanvasItem):
		var canvas_payload := base_payload.duplicate(true)
		canvas_payload["requested_space"] = "viewport_2d"
		canvas_payload["actual_space"] = ""
		canvas_payload["coordinate_space"] = ""
		canvas_payload["rect_source"] = "none"
		canvas_payload["missing_evidence"] = "viewport_rect"
		canvas_payload["actual"] = {
			"target_class": node_a.get_class(),
			"other_class": node_b.get_class(),
			"target_is_canvas_item": node_a is CanvasItem,
			"other_is_canvas_item": node_b is CanvasItem,
		}
		canvas_payload["expected"] = "CanvasItem targets with measurable viewport-space geometry"
		canvas_payload["remediation"] = _visual_rect_remediation()
		canvas_payload["artifacts"] = {}
		return _owner.get_ref()._error("unsupported_visual_assertion", "render-order assertions require CanvasItem targets", canvas_payload)
	var rect_evidence_a := _visual_rect_for_node(node_a, command_name, a, params, left)
	if not bool(rect_evidence_a.get("ok", false)):
		return rect_evidence_a
	var rect_evidence_b := _visual_rect_for_node(node_b, command_name, b, params, right)
	if not bool(rect_evidence_b.get("ok", false)):
		return rect_evidence_b
	var rect_a: Rect2 = rect_evidence_a.get("rect")
	var rect_b: Rect2 = rect_evidence_b.get("rect")
	# The geometry evidence (target_rect/other_rect below) stays in canvas space,
	# but pixel sampling and overlays index the window-resolution screenshot, so
	# convert to screenshot space before touching image pixels.
	var canvas_to_screenshot := _canvas_to_screenshot_transform()
	var rect_a_screenshot: Rect2 = canvas_to_screenshot * rect_a
	var rect_b_screenshot: Rect2 = canvas_to_screenshot * rect_b
	var overlaps := rect_a.intersects(rect_b)
	if require_overlap and not overlaps:
		var no_overlap_payload := base_payload.duplicate(true)
		no_overlap_payload["target_rect"] = _rect_payload(rect_a)
		no_overlap_payload["other_rect"] = _rect_payload(rect_b)
		no_overlap_payload["target_rect_source"] = rect_evidence_a.get("rect_source", "")
		no_overlap_payload["other_rect_source"] = rect_evidence_b.get("rect_source", "")
		no_overlap_payload["target_missing_evidence"] = rect_evidence_a.get("missing_evidence", "")
		no_overlap_payload["other_missing_evidence"] = rect_evidence_b.get("missing_evidence", "")
		no_overlap_payload["requested_space"] = "viewport_2d"
		no_overlap_payload["target_actual_space"] = rect_evidence_a.get("coordinate_space", "")
		no_overlap_payload["other_actual_space"] = rect_evidence_b.get("coordinate_space", "")
		no_overlap_payload["actual"] = {"target_rect": _rect_payload(rect_a), "other_rect": _rect_payload(rect_b), "overlap": false}
		no_overlap_payload["expected"] = "overlapping viewport-space target and other rects"
		no_overlap_payload["remediation"] = "use targets with overlapping rendered viewport rects or set require_overlap false when canvas ordering is sufficient"
		no_overlap_payload["artifacts"] = {}
		no_overlap_payload["evidence"] = requested_evidence
		return _owner.get_ref()._error("unsupported_visual_assertion", "render-order assertions require overlapping CanvasItem targets", no_overlap_payload)
	var order := _snapshot_pixel_order_evidence(node_a as CanvasItem, node_b as CanvasItem, rect_a_screenshot, rect_b_screenshot, params)
	if not bool(order.get("ok", false)):
		order = _canvas_order_evidence(node_a as CanvasItem, node_b as CanvasItem)
	var visual_dir := str(params.get("visual_dir", ""))
	var payload := base_payload.duplicate(true)
	payload["target_rect"] = _rect_payload(rect_a)
	payload["other_rect"] = _rect_payload(rect_b)
	payload["overlap_rect"] = _rect_payload(rect_a.intersection(rect_b)) if overlaps else []
	payload["target_rect_source"] = rect_evidence_a.get("rect_source", "")
	payload["other_rect_source"] = rect_evidence_b.get("rect_source", "")
	payload["target_coordinate_space"] = rect_evidence_a.get("coordinate_space", "")
	payload["other_coordinate_space"] = rect_evidence_b.get("coordinate_space", "")
	payload["target_missing_evidence"] = rect_evidence_a.get("missing_evidence", "")
	payload["other_missing_evidence"] = rect_evidence_b.get("missing_evidence", "")
	payload["evidence_mode"] = "canvas_order"
	payload["requested_evidence"] = requested_evidence
	payload["require_overlap"] = require_overlap
	payload["order_evidence"] = order
	payload["artifacts"] = {}
	if visual_dir != "" and not visual_dir.contains("qa/baselines"):
		var dir_err := DirAccess.make_dir_recursive_absolute(visual_dir)
		if dir_err == OK:
			var safe_name := _sanitize_visual_name("%s_%s_%s" % [command_name, a, b])
			var screenshot_path := "%s/%s_order.png" % [visual_dir, safe_name]
			var overlay_path := "%s/%s_order_overlay.png" % [visual_dir, safe_name]
			var capture := await _capture_visual_image(screenshot_path)
			if bool(capture.get("ok", false)):
				var image: Image = capture.get("image")
				var overlay_error := _write_rect_overlay(image, rect_a_screenshot.intersection(rect_b_screenshot) if overlaps else rect_a_screenshot, overlay_path)
				payload["screenshot"] = screenshot_path
				payload["artifacts"] = {"screenshot": screenshot_path, "overlay": overlay_path}
				var artifact_failures: Array = capture.get("artifact_failures", []).duplicate(true)
				if overlay_error != OK:
					artifact_failures.append(_visual_artifact_write_failure(overlay_path, overlay_error))
				if not artifact_failures.is_empty():
					payload["capture_warning"] = {"type": "screenshot_write_failed", "message": "optional render-order diagnostic artifacts could not be written", "artifact_failures": artifact_failures}
			else:
				payload["capture_warning"] = _visual_capture_warning(capture, visual_dir, screenshot_path)
		else:
			payload["capture_warning"] = {"type": "screenshot_write_failed", "actual": dir_err, "visual_dir": visual_dir}
	if not bool(order.get("ok", false)):
		payload["reason"] = str(order.get("reason", "ambiguous_canvas_order"))
		return _owner.get_ref()._error("unsupported_visual_assertion", "render-order evidence was not conclusive", payload)
	var target_above := bool(order.get("target_above_other", false))
	var passes := target_above if expect_above else not target_above
	if passes:
		return _owner.get_ref()._ok(payload)
	payload["reason"] = "target_rendered_below_other" if expect_above else "target_rendered_above_other"
	return _owner.get_ref()._error("render_order_mismatch", "target render order did not match expectation", payload)


func _validate_visual_pixel_params(params: Dictionary, command_name: String, require_threshold: bool) -> Dictionary:
	var target := str(params.get("target", "")).strip_edges()
	var snapshot := _visual_snapshot(params).strip_edges()
	var base := {"command": command_name, "target": target, "snapshot": snapshot}
	if target == "":
		base["field"] = "target"
		base["actual"] = params.get("target", null)
		return _owner.get_ref()._error("invalid_assertion_args", "visual assertions require a non-empty target", base)
	if snapshot == "":
		base["field"] = "snapshot"
		base["actual"] = params.get("snapshot", null)
		return _owner.get_ref()._error("invalid_assertion_args", "visual assertions require a non-empty snapshot", base)
	var has_min_ratio := params.has("min_ratio") or params.has("threshold")
	var has_min_count := params.has("min_count")
	if require_threshold and not has_min_ratio and not has_min_count:
		base["field"] = "threshold"
		base["actual"] = {"min_ratio": null, "min_count": null}
		return _owner.get_ref()._error("invalid_assertion_args", "visible_pixels requires min_ratio or min_count", base)
	var threshold := {}
	var result := {
		"ok": true,
		"target": target,
		"snapshot": snapshot,
		"has_min_ratio": has_min_ratio,
		"has_min_count": has_min_count,
		"threshold": threshold,
	}
	if has_min_ratio:
		var ratio_value = params.get("min_ratio", params.get("threshold", null))
		if typeof(ratio_value) != TYPE_FLOAT and typeof(ratio_value) != TYPE_INT:
			base["field"] = "min_ratio"
			base["actual"] = ratio_value
			return _owner.get_ref()._error("invalid_assertion_args", "visual assertion min_ratio must be a number between 0.0 and 1.0", base)
		var ratio := float(ratio_value)
		if is_nan(ratio) or is_inf(ratio) or ratio < 0.0 or ratio > 1.0:
			base["field"] = "min_ratio"
			base["actual"] = ratio_value
			return _owner.get_ref()._error("invalid_assertion_args", "visual assertion min_ratio must be a number between 0.0 and 1.0", base)
		threshold["min_ratio"] = ratio
		result["min_ratio"] = ratio
	if has_min_count:
		var count_value = params.get("min_count", null)
		if (typeof(count_value) != TYPE_INT and typeof(count_value) != TYPE_FLOAT) or int(count_value) < 0 or float(count_value) != float(int(count_value)):
			base["field"] = "min_count"
			base["actual"] = count_value
			return _owner.get_ref()._error("invalid_assertion_args", "visual assertion min_count must be a non-negative integer", base)
		threshold["min_count"] = int(count_value)
		result["min_count"] = int(count_value)
	return result


func _validate_visual_order_params(params: Dictionary, command_name: String) -> Dictionary:
	var target := str(params.get("a", params.get("target", ""))).strip_edges()
	var other := str(params.get("b", params.get("other", ""))).strip_edges()
	if other == "":
		other = str(params.get("peer", "")).strip_edges()
	var snapshot := _visual_snapshot(params).strip_edges()
	var base := {"command": command_name, "target": target, "other": other, "snapshot": snapshot}
	if target == "":
		base["field"] = "target"
		base["actual"] = params.get("target", params.get("a", null))
		return _owner.get_ref()._error("invalid_assertion_args", "render-order assertions require a non-empty target", base)
	if other == "":
		base["field"] = "other"
		base["actual"] = params.get("other", params.get("peer", params.get("b", null)))
		return _owner.get_ref()._error("invalid_assertion_args", "render-order assertions require a non-empty other target", base)
	if snapshot == "":
		base["field"] = "snapshot"
		base["actual"] = params.get("snapshot", null)
		return _owner.get_ref()._error("invalid_assertion_args", "render-order assertions require a non-empty snapshot", base)
	return {"ok": true, "target": target, "other": other, "snapshot": snapshot}


func _visual_probe(target: String, params: Dictionary, command_name: String) -> Dictionary:
	var resolution = _owner.get_ref()._require_target(target)
	if not bool(resolution.get("ok", false)):
		return _visual_error_with_context(resolution, target, params, command_name)
	var node: Node = resolution.get("node")
	var rect_result := _visual_rect_for_node(node, command_name, target, params, resolution)
	if not bool(rect_result.get("ok", false)):
		return rect_result
	var rect: Rect2 = rect_result.get("rect")
	if rect.size.x <= 0.0 or rect.size.y <= 0.0:
		return _owner.get_ref()._error(
			"unsupported_visual_assertion",
			"target did not expose a measurable viewport rect",
			_visual_probe_failure_payload(
				command_name,
				target,
				params,
				resolution,
				rect_result,
				rect_result.get("missing_evidence", "viewport_rect"),
				_rect_payload(rect),
				"viewport-space rect with positive width and height",
				rect_result.get("remediation", _visual_rect_remediation()),
			),
		)
	var visual_dir := str(params.get("visual_dir", ""))
	if visual_dir == "" or visual_dir.contains("qa/baselines"):
		return _owner.get_ref()._error("invalid_visual_dir", "visual assertions require safe visual_dir outside qa/baselines", {"command": command_name, "target": target, "snapshot": _visual_snapshot(params), "visual_dir": visual_dir})
	var dir_err := DirAccess.make_dir_recursive_absolute(visual_dir)
	if dir_err != OK:
		return _owner.get_ref()._error("screenshot_write_failed", "visual artifact directory could not be created", {"command": command_name, "target": target, "snapshot": _visual_snapshot(params), "visual_dir": visual_dir, "actual": dir_err})
	var safe_name := _sanitize_visual_name("%s_%s_%s" % [command_name, _visual_snapshot(params), target])
	var before_path := "%s/%s_original.png" % [visual_dir, safe_name]
	var hidden_path := "%s/%s_hidden.png" % [visual_dir, safe_name]
	var visibility_state := _visual_visibility_state(node)
	if not bool(visibility_state.get("ok", false)):
		return _owner.get_ref()._error(
			"unsupported_visual_assertion",
			"visual assertions require CanvasItem visibility or qa_set_visual_visible",
			_visual_probe_failure_payload(
				command_name,
				target,
				params,
				resolution,
				rect_result,
				visibility_state.get("missing_evidence", "visibility_control"),
				visibility_state.get("actual", {}),
				"CanvasItem.visible or qa_set_visual_visible(visible) support",
				visibility_state.get("remediation", _visual_visibility_remediation()),
			),
		)
	var before_capture := await _capture_visual_image(before_path)
	if not bool(before_capture.get("ok", false)):
		return _visual_capture_error(target, resolution, visual_dir, before_capture, params, command_name)
	if not _safe_set_visual_visible(node, false):
		return _target_freed_visual_probe_error(target, resolution, params, command_name, {"original": before_path}, before_capture.get("artifact_failures", []))
	var hidden_capture := await _capture_visual_image(hidden_path)
	var artifact_failures: Array = before_capture.get("artifact_failures", []).duplicate(true)
	artifact_failures.append_array(hidden_capture.get("artifact_failures", []))
	hidden_capture["artifact_failures"] = artifact_failures
	if not _safe_restore_visual_visible(node, visibility_state):
		return _target_freed_visual_probe_error(target, resolution, params, command_name, {"original": before_path, "target_hidden": hidden_path}, artifact_failures)
	await _runtime.process_frames(1)
	if not bool(hidden_capture.get("ok", false)):
		return _visual_capture_error(target, resolution, visual_dir, hidden_capture, params, command_name)
	var before_img: Image = before_capture.get("image")
	var hidden_img: Image = hidden_capture.get("image")
	if before_img.get_width() != hidden_img.get_width() or before_img.get_height() != hidden_img.get_height():
		return _owner.get_ref()._error("screenshot_capture_failed", "visual assertions require matching real viewport screenshot image evidence", {"command": command_name, "target": target, "snapshot": _visual_snapshot(params), "resolved": resolution.get("resolved", ""), "resolved_path": resolution.get("resolved", ""), "visual_dir": visual_dir, "display_server": DisplayServer.get_name(), "headless": OS.has_feature("headless"), "attempted_screenshot_path": hidden_path, "capture_details": {"original_size": [before_img.get_width(), before_img.get_height()], "hidden_size": [hidden_img.get_width(), hidden_img.get_height()]}, "actual": {"original_size": [before_img.get_width(), before_img.get_height()], "hidden_size": [hidden_img.get_width(), hidden_img.get_height()]}, "expected": "matching real rendered viewport pixel screenshots", "remediation": _visual_capture_remediation(), "artifacts": {"original": before_path, "target_hidden": hidden_path}, "artifact_failures": artifact_failures})
	# Target rects come from get_global_rect()/qa_visual_rect() in the 2D canvas
	# coordinate space, but screenshots are captured at the OS window resolution.
	# When content scaling maps canvas != window, canvas coords index the wrong
	# screenshot pixels, so convert the rect into screenshot space before sampling.
	var canvas_to_screenshot := _canvas_to_screenshot_transform()
	var image_size := Vector2(before_img.get_width(), before_img.get_height())
	var geometry_error := _screenshot_geometry_mismatch_error(canvas_to_screenshot, image_size, command_name, target, params, resolution, before_path, hidden_path)
	if not geometry_error.is_empty():
		geometry_error["error"]["data"]["artifact_failures"] = artifact_failures
		return geometry_error
	var rect_screenshot: Rect2 = canvas_to_screenshot * rect
	var mask_path := "%s/%s_mask.png" % [visual_dir, safe_name]
	var overlay_path := "%s/%s_overlay.png" % [visual_dir, safe_name]
	var metrics := _write_visual_diagnostics(before_img, hidden_img, rect_screenshot, mask_path, overlay_path)
	artifact_failures.append_array(metrics.get("artifact_failures", []))
	return {
		"ok": true,
		"command": command_name,
		"target": target,
		"snapshot": _visual_snapshot(params),
		"resolved": resolution.get("resolved", ""),
		"visible_pixel_count": metrics.get("changed", 0),
		"visible_pixel_ratio": metrics.get("ratio", 0.0),
		"evidence_mode": metrics.get("mode", "pixel_delta"),
		"target_rect": _rect_payload(rect),
		"target_rect_screenshot": _rect_payload(rect_screenshot),
		"canvas_to_screenshot": _canvas_to_screenshot_payload(canvas_to_screenshot),
		"screenshot_size": [before_img.get_width(), before_img.get_height()],
		"rect_source": rect_result.get("rect_source", ""),
		"coordinate_space": rect_result.get("coordinate_space", "viewport_2d"),
		"missing_evidence": rect_result.get("missing_evidence", ""),
		"remediation": rect_result.get("remediation", ""),
		"screenshot": before_path,
		"artifacts": {"original": before_path, "target_hidden": hidden_path, "mask": mask_path, "overlay": overlay_path},
		"artifact_failures": artifact_failures,
	}


func _visual_artifact_write_failure(path: String, error_code: int) -> Dictionary:
	return {"type": "screenshot_write_failed", "message": "visual diagnostic PNG could not be written", "attempted_screenshot_path": path, "expected": "writable visual diagnostic PNG path", "actual": error_code, "screenshot_error": error_code}


func _visual_required_artifact_result(payload: Dictionary) -> Dictionary:
	var failures: Array = payload.get("artifact_failures", [])
	if failures.is_empty():
		return _owner.get_ref()._ok(payload)
	var failure: Dictionary = failures[0]
	payload["attempted_screenshot_path"] = failure.get("attempted_screenshot_path", "")
	payload["screenshot_error"] = failure.get("screenshot_error", ERR_CANT_CREATE)
	return _owner.get_ref()._error("screenshot_write_failed", "required visual diagnostic PNG could not be written", payload)


func _visual_rect_for_node(node: Node, command_name: String, target: String, params: Dictionary, resolution: Dictionary) -> Dictionary:
	var context := {"command": command_name, "target": target, "snapshot": _visual_snapshot(params), "resolved": resolution.get("resolved", ""), "resolved_path": resolution.get("resolved", ""), "requested_space": "viewport_2d", "actual_space": "", "coordinate_space": "", "artifacts": {}}
	if node == null:
		context["missing_evidence"] = "resolved_target"
		context["actual"] = null
		context["expected"] = "resolved live target node"
		context["remediation"] = "resolve the assertion target to a live node before requesting viewport pixel evidence"
		return _owner.get_ref()._error("unsupported_visual_assertion", "visual assertions require a resolved target node", context)
	var snapshot_evidence := QaSnapshot.visual_evidence_for_node(node)
	var snapshot_rect := _rect_from_visual_evidence(snapshot_evidence)
	if bool(snapshot_rect.get("ok", false)):
		return _normalized_visual_rect_evidence(snapshot_rect.get("rect"), snapshot_evidence.get("rect_source", ""), snapshot_evidence.get("coordinate_space", "viewport_2d"))
	if str(snapshot_evidence.get("unsupported_reason", "")) == "unsupported_coordinate_space":
		var space_payload := context.duplicate(true)
		space_payload["requested_space"] = "viewport_2d"
		space_payload["actual_space"] = snapshot_evidence.get("coordinate_space", "")
		space_payload["coordinate_space"] = snapshot_evidence.get("coordinate_space", "")
		space_payload["actual"] = snapshot_evidence.get("actual", null)
		space_payload["expected"] = "viewport_2d"
		space_payload["missing_evidence"] = "coordinate_conversion"
		space_payload["remediation"] = _visual_rect_remediation()
		return _owner.get_ref()._error("unsupported_coordinate_space", "qa_visual_rect must return viewport_2d evidence", space_payload)
	context["rect_source"] = snapshot_evidence.get("rect_source", "")
	context["coordinate_space"] = snapshot_evidence.get("coordinate_space", "")
	context["actual_space"] = snapshot_evidence.get("coordinate_space", "")
	context["missing_evidence"] = snapshot_evidence.get("unsupported_reason", "viewport_rect")
	context["actual"] = snapshot_evidence.get("actual", {"class": node.get_class(), "has_qa_visual_rect": node.has_method("qa_visual_rect") or node.has_meta("qa_visual_rect")})
	context["expected"] = "measurable viewport-space geometry"
	context["remediation"] = _visual_rect_remediation()
	return _owner.get_ref()._error("unsupported_visual_assertion", "target did not expose measurable viewport geometry", context)


func _explicit_visual_rect(node: Node) -> Dictionary:
	var value: Variant = null
	if node.has_method("qa_visual_rect"):
		value = node.call("qa_visual_rect")
	elif node.has_meta("qa_visual_rect"):
		value = node.get_meta("qa_visual_rect")
	else:
		return {"ok": false, "missing": true}
	var parsed := _parse_visual_rect(value)
	if not bool(parsed.get("ok", false)):
		parsed["actual"] = value
		return parsed
	parsed["rect_source"] = "qa_visual_rect"
	return parsed


func _parse_visual_rect(value: Variant) -> Dictionary:
	if value is Rect2:
		return {"ok": true, "rect": value, "coordinate_space": "viewport_2d"}
	if value is Array:
		var values: Array = value
		if values.size() >= 4:
			return {"ok": true, "rect": Rect2(float(values[0]), float(values[1]), float(values[2]), float(values[3])), "coordinate_space": "viewport_2d"}
	if value is Dictionary:
		var rect: Dictionary = value
		var coordinate_space := str(rect.get("coordinate_space", rect.get("space", "viewport_2d"))).strip_edges()
		if coordinate_space == "":
			coordinate_space = "viewport_2d"
		if coordinate_space != "viewport_2d":
			return {"ok": false, "missing": false, "error_type": "unsupported_coordinate_space", "coordinate_space": coordinate_space, "actual_space": coordinate_space}
		if rect.has("rect"):
			var parsed_rect := _parse_visual_rect(rect.get("rect"))
			if not bool(parsed_rect.get("ok", false)):
				parsed_rect["coordinate_space"] = coordinate_space
				parsed_rect["actual_space"] = parsed_rect.get("actual_space", coordinate_space)
				return parsed_rect
			parsed_rect["coordinate_space"] = coordinate_space
			return parsed_rect
		if rect.has("x") and rect.has("y") and (rect.has("width") or rect.has("w")) and (rect.has("height") or rect.has("h")):
			return {"ok": true, "rect": Rect2(float(rect.get("x")), float(rect.get("y")), float(rect.get("width", rect.get("w"))), float(rect.get("height", rect.get("h")))), "coordinate_space": coordinate_space}
		if rect.has("position") and rect.has("size") and rect.get("position") is Array and rect.get("size") is Array:
			var position: Array = rect.get("position")
			var size: Array = rect.get("size")
			if position.size() >= 2 and size.size() >= 2:
				return {"ok": true, "rect": Rect2(float(position[0]), float(position[1]), float(size[0]), float(size[1])), "coordinate_space": coordinate_space}
	return {"ok": false, "missing": false}


func _visual_visibility_state(node: Node) -> Dictionary:
	if node == null or not is_instance_valid(node):
		return {"ok": false, "reason": "target_freed", "missing_evidence": "visibility_control", "actual": "target_freed", "remediation": _visual_visibility_remediation()}
	if node is CanvasItem:
		return {"ok": true, "mode": "canvas_item", "previous": bool((node as CanvasItem).visible)}
	if node.has_method("qa_set_visual_visible"):
		var previous := true
		if node.has_method("qa_visual_visible"):
			previous = bool(node.call("qa_visual_visible"))
		elif node.has_meta("qa_visual_visible"):
			previous = bool(node.get_meta("qa_visual_visible"))
		return {"ok": true, "mode": "qa_set_visual_visible", "previous": previous}
	return {"ok": false, "reason": "missing_visibility_control", "missing_evidence": "visibility_control", "actual": {"class": node.get_class(), "is_canvas_item": false, "has_qa_set_visual_visible": false}, "remediation": _visual_visibility_remediation()}


func _visual_visibility_remediation() -> String:
	return "make the visual target a CanvasItem or expose qa_set_visual_visible(visible) with optional qa_visual_visible evidence so pixel-delta assertions can hide and restore it"


func _set_visual_visible(node: Node, visible: bool) -> void:
	if node is CanvasItem:
		(node as CanvasItem).visible = visible
	elif node.has_method("qa_set_visual_visible"):
		node.call("qa_set_visual_visible", visible)


func _restore_visual_visible(node: Node, visibility_state: Dictionary) -> void:
	_set_visual_visible(node, bool(visibility_state.get("previous", true)))


func _safe_set_visual_visible(node: Node, visible: bool) -> bool:
	if node == null or not is_instance_valid(node):
		return false
	_set_visual_visible(node, visible)
	return is_instance_valid(node)


func _safe_restore_visual_visible(node: Node, visibility_state: Dictionary) -> bool:
	if node == null or not is_instance_valid(node):
		return false
	_restore_visual_visible(node, visibility_state)
	return is_instance_valid(node)


func _capture_visual_image(path: String) -> Dictionary:
	# Keep the exact captured pixels for analysis instead of decoding the PNG
	# that was just written. Preserve capture_screenshot's settling and strictness.
	await _root_window.get_tree().process_frame
	await _root_window.get_tree().process_frame
	RenderingServer.force_draw(false)
	var image: Image = QaSnapshot._capture_viewport_image(_root_window)
	var capture_result := {"artifacts": {"screenshot": path}}
	if image == null or image.get_width() <= 0 or image.get_height() <= 0:
		return _visual_image_capture_failure(path, "visual screenshot viewport pixels were unavailable", "headless_screenshot_unsupported" if DisplayServer.get_name() == "headless" else "non_loadable_screenshot_image", null, capture_result)
	var screenshot_error: int = QaSnapshot._save_captured_viewport_screenshot(_root_window, image, path, false)
	var artifact_failures: Array = []
	if screenshot_error != OK:
		artifact_failures.append(_visual_artifact_write_failure(path, screenshot_error))
	capture_result["artifact_failures"] = artifact_failures
	if image.get_format() != Image.FORMAT_RGBA8:
		image.convert(Image.FORMAT_RGBA8)
	if OS.has_feature("headless") or DisplayServer.get_name() == "headless":
		return _visual_image_capture_failure(path, "visual assertions cannot use headless diagnostic screenshot output", "headless_screenshot_unsupported", image, capture_result)
	if _visual_image_is_low_information(image):
		return _visual_image_capture_failure(path, "visual screenshot did not contain real rendered viewport pixels", "blank_or_fallback_screenshot_image", image, capture_result)
	return {"ok": true, "image": image, "artifacts": capture_result.get("artifacts", {}), "artifact_failures": artifact_failures}


func _visual_image_capture_failure(path: String, message: String, reason: String, image: Image, capture_result: Dictionary) -> Dictionary:
	var actual := {
		"path": path,
		"reason": reason,
		"display_server": DisplayServer.get_name(),
		"headless": OS.has_feature("headless"),
	}
	if image != null:
		actual["image_size"] = [image.get_width(), image.get_height()]
	return {
		"ok": false,
		"failures": [
			{
				"type": "screenshot_capture_failed",
				"message": message,
				"expected": "real rendered viewport pixels from the active Godot display backend",
				"actual": actual,
				"attempted_screenshot_path": path,
				"capture_reason": reason,
				"display_server": DisplayServer.get_name(),
				"headless": OS.has_feature("headless"),
			},
		],
		"artifacts": capture_result.get("artifacts", {"screenshot": path}),
		"artifact_failures": capture_result.get("artifact_failures", []),
	}


func _visual_capture_error(target: String, resolution: Dictionary, visual_dir: String, capture: Dictionary, params := {}, command_name := "") -> Dictionary:
	var failures = capture.get("failures", [])
	var failure := {}
	if failures is Array and not failures.is_empty() and failures[0] is Dictionary:
		failure = (failures[0] as Dictionary).duplicate(true)
	var artifacts: Dictionary = capture.get("artifacts", {}) if capture.get("artifacts", {}) is Dictionary else {}
	var attempted_path := str(failure.get("attempted_screenshot_path", artifacts.get("screenshot", "")))
	if attempted_path == "" and failure.get("actual", null) is String:
		attempted_path = str(failure.get("actual"))
	var capture_details := failure.duplicate(true)
	capture_details["capture_result"] = capture.duplicate(true)
	return _owner.get_ref()._error(
		"screenshot_capture_failed",
		str(failure.get("message", "visual assertions require real rendered viewport pixel evidence")),
		{
			"command": str(command_name),
			"target": target,
			"snapshot": _visual_snapshot(params),
			"resolved": resolution.get("resolved", ""),
			"resolved_path": resolution.get("resolved", ""),
			"visual_dir": visual_dir,
			"display_server": DisplayServer.get_name(),
			"headless": OS.has_feature("headless"),
			"attempted_screenshot_path": attempted_path,
			"capture_details": capture_details,
			"actual": failure.get("actual", capture),
			"expected": "real rendered viewport pixels from the active Godot display backend",
			"remediation": _visual_capture_remediation(),
			"artifacts": artifacts,
			"artifact_failures": capture.get("artifact_failures", []),
		},
	)


func _visual_capture_warning(capture: Dictionary, visual_dir: String = "", attempted_screenshot_path: String = "") -> Dictionary:
	var failures = capture.get("failures", [])
	var failure := {}
	if failures is Array and not failures.is_empty() and failures[0] is Dictionary:
		failure = (failures[0] as Dictionary).duplicate(true)
	var artifacts: Dictionary = capture.get("artifacts", {}) if capture.get("artifacts", {}) is Dictionary else {}
	var attempted_path := attempted_screenshot_path
	if attempted_path == "":
		attempted_path = str(failure.get("actual", artifacts.get("screenshot", "")))
	return {
		"type": "screenshot_capture_failed",
		"message": str(failure.get("message", "visual assertions require real rendered viewport pixel evidence")),
		"visual_dir": visual_dir,
		"display_server": DisplayServer.get_name(),
		"headless": OS.has_feature("headless"),
		"attempted_screenshot_path": attempted_path,
		"capture_details": failure if not failure.is_empty() else capture,
		"actual": failure.get("actual", capture),
		"expected": "real rendered viewport pixels from the active Godot display backend",
		"remediation": _visual_capture_remediation(),
		"artifacts": artifacts,
	}


func _visual_capture_remediation() -> String:
	return "visual-pixel assertions require real rendered viewport pixels; run with a display backend that exposes screenshots, or use non-visual state/probe assertions when pixel evidence is unavailable"


func _visual_image_is_low_information(image: Image) -> bool:
	var width := image.get_width()
	var height := image.get_height()
	if width <= 0 or height <= 0:
		return true
	var first := image.get_pixel(0, 0)
	var visible_count := 0
	var different_count := 0
	var step_x := maxi(1, int(floor(float(width) / 32.0)))
	var step_y := maxi(1, int(floor(float(height) / 32.0)))
	for y in range(0, height, step_y):
		for x in range(0, width, step_x):
			var color := image.get_pixel(x, y)
			if color.a > 0.01:
				visible_count += 1
			if _pixels_differ(first, color):
				different_count += 1
	return visible_count == 0 or different_count == 0


func _visual_probe_failure_payload(
	command_name: String,
	target: String,
	params: Dictionary,
	resolution: Dictionary,
	rect_result: Dictionary,
	missing_evidence: String,
	actual: Variant,
	expected: Variant,
	remediation: String,
) -> Dictionary:
	return {
		"command": command_name,
		"target": target,
		"snapshot": _visual_snapshot(params),
		"resolved": resolution.get("resolved", ""),
		"resolved_path": resolution.get("resolved", ""),
		"rect_source": rect_result.get("rect_source", ""),
		"coordinate_space": rect_result.get("coordinate_space", "viewport_2d"),
		"requested_space": "viewport_2d",
		"actual_space": rect_result.get("coordinate_space", "viewport_2d"),
		"missing_evidence": missing_evidence,
		"actual": actual,
		"expected": expected,
		"remediation": remediation,
		"artifacts": {},
	}


func _target_freed_visual_probe_error(target: String, resolution: Dictionary, params: Dictionary, command_name: String, artifacts: Dictionary, artifact_failures: Array = []) -> Dictionary:
	return _owner.get_ref()._error(
		"target_freed_during_visual_probe",
		"visual assertion target was freed while screenshot evidence was being captured",
		{
			"command": command_name,
			"target": target,
			"snapshot": _visual_snapshot(params),
			"resolved": resolution.get("resolved", ""),
			"artifacts": artifacts,
			"artifact_failures": artifact_failures,
		},
	)


func _pixels_differ(a: Color, b: Color) -> bool:
	return (
		abs(a.r - b.r) > 0.01
		or abs(a.g - b.g) > 0.01
		or abs(a.b - b.b) > 0.01
		or abs(a.a - b.a) > 0.01
	)


func _write_visual_diagnostics(before: Image, hidden: Image, rect: Rect2, mask_path: String, overlay_path: String) -> Dictionary:
	var width := before.get_width()
	var height := before.get_height()
	var mask := Image.create(width, height, false, Image.FORMAT_RGBA8)
	var overlay := Image.create(width, height, false, Image.FORMAT_RGBA8)
	overlay.blit_rect(before, Rect2i(0, 0, width, height), Vector2i.ZERO)
	var x0 := maxi(0, int(floor(rect.position.x)))
	var y0 := maxi(0, int(floor(rect.position.y)))
	var x1 := mini(width, int(ceil(rect.end.x)))
	var y1 := mini(height, int(ceil(rect.end.y)))
	var total := maxi((x1 - x0) * (y1 - y0), 1)
	var changed := 0
	for y in range(y0, y1):
		for x in range(x0, x1):
			if _pixels_differ(before.get_pixel(x, y), hidden.get_pixel(x, y)):
				changed += 1
				mask.set_pixel(x, y, Color(1.0, 1.0, 1.0, 1.0))
				var base := overlay.get_pixel(x, y)
				overlay.set_pixel(x, y, base.lerp(Color(1.0, 0.0, 0.0, 1.0), 0.55))
	var failures: Array = []
	var mask_error := mask.save_png(mask_path)
	if mask_error != OK:
		failures.append(_visual_artifact_write_failure(mask_path, mask_error))
	var overlay_error := overlay.save_png(overlay_path)
	if overlay_error != OK:
		failures.append(_visual_artifact_write_failure(overlay_path, overlay_error))
	return {"changed": changed, "ratio": float(changed) / float(total), "mode": "pixel_delta", "artifact_failures": failures}


func _write_rect_overlay(source: Image, rect: Rect2, overlay_path: String) -> int:
	var width := source.get_width()
	var height := source.get_height()
	var overlay := Image.create(width, height, false, Image.FORMAT_RGBA8)
	overlay.blit_rect(source, Rect2i(0, 0, width, height), Vector2i.ZERO)
	var x0 := maxi(0, int(floor(rect.position.x)))
	var y0 := maxi(0, int(floor(rect.position.y)))
	var x1 := mini(width, int(ceil(rect.end.x)))
	var y1 := mini(height, int(ceil(rect.end.y)))
	for y in range(y0, y1):
		for x in range(x0, x1):
			if x == x0 or x == x1 - 1 or y == y0 or y == y1 - 1:
				overlay.set_pixel(x, y, Color(1.0, 0.0, 0.0, 1.0))
			else:
				var base := overlay.get_pixel(x, y)
				overlay.set_pixel(x, y, base.lerp(Color(1.0, 0.0, 0.0, 1.0), 0.25))
	return overlay.save_png(overlay_path)


func _canvas_order_evidence(target: CanvasItem, other: CanvasItem) -> Dictionary:
	if not target.is_visible_in_tree() or not other.is_visible_in_tree():
		return {"ok": false, "reason": "targets_must_be_visible_in_tree"}
	var target_layer := _canvas_layer_index(target)
	var other_layer := _canvas_layer_index(other)
	if target_layer != other_layer:
		return {
			"ok": true,
			"mode": "canvas_layer",
			"target_above_other": target_layer > other_layer,
			"target_layer": target_layer,
			"other_layer": other_layer,
		}
	var target_z := _effective_canvas_z(target)
	var other_z := _effective_canvas_z(other)
	if target_z != other_z:
		return {
			"ok": true,
			"mode": "z_index",
			"target_above_other": target_z > other_z,
			"target_z": target_z,
			"other_z": other_z,
			"target_z_as_relative": bool(target.z_as_relative),
			"other_z_as_relative": bool(other.z_as_relative),
			"target_layer": target_layer,
			"other_layer": other_layer,
		}
	if target.get_parent() != null and target.get_parent() == other.get_parent():
		return {
			"ok": true,
			"mode": "direct_sibling_index",
			"target_above_other": target.get_index() > other.get_index(),
			"target_index": target.get_index(),
			"other_index": other.get_index(),
			"target_z": target_z,
			"other_z": other_z,
			"target_z_as_relative": bool(target.z_as_relative),
			"other_z_as_relative": bool(other.z_as_relative),
			"target_layer": target_layer,
			"other_layer": other_layer,
		}
	return {
		"ok": false,
		"reason": "same_z_order_without_direct_sibling_evidence",
		"target_z": target_z,
		"other_z": other_z,
		"target_z_as_relative": bool(target.z_as_relative),
		"other_z_as_relative": bool(other.z_as_relative),
		"target_layer": target_layer,
		"other_layer": other_layer,
	}


func _canvas_layer_index(item: CanvasItem) -> int:
	var current: Node = item
	while current != null:
		if current is CanvasLayer:
			return int((current as CanvasLayer).layer)
		current = current.get_parent()
	return 0


func _effective_canvas_z(item: CanvasItem) -> int:
	var total := 0
	var current: Node = item
	while current != null:
		if not (current is CanvasItem):
			break
		var canvas_item := current as CanvasItem
		total += int(canvas_item.z_index)
		if not canvas_item.z_as_relative:
			return total
		current = current.get_parent()
	return total


func _visual_snapshot(params) -> String:
	if params is Dictionary:
		return str((params as Dictionary).get("snapshot", ""))
	return ""


func _visual_error_with_context(result: Dictionary, target: String, params: Dictionary, command_name: String) -> Dictionary:
	var error := (result.get("error", {}) as Dictionary).duplicate(true)
	var data := (error.get("data", {}) as Dictionary).duplicate(true)
	if not data.has("command"):
		data["command"] = command_name
	if not data.has("target"):
		data["target"] = target
	if not data.has("snapshot"):
		data["snapshot"] = _visual_snapshot(params)
	error["data"] = data
	return {"ok": false, "error": error}


func _snapshot_pixel_order_evidence(target_item: CanvasItem, other_item: CanvasItem, target_rect: Rect2, other_rect: Rect2, params: Dictionary) -> Dictionary:
	var snapshot := _visual_snapshot(params)
	var visual_dir := str(params.get("visual_dir", ""))
	if snapshot == "" or visual_dir == "":
		return {"ok": false, "reason": "missing_snapshot_pixel_evidence"}
	if not (target_item is ColorRect and other_item is ColorRect):
		return {"ok": false, "reason": "snapshot_pixel_evidence_requires_color_rects"}
	var overlap := target_rect.intersection(other_rect)
	if overlap.size.x <= 0.0 or overlap.size.y <= 0.0:
		return {"ok": false, "reason": "no_overlap"}
	var snapshot_path := "%s/snapshots/%s/screenshot.png" % [visual_dir.get_base_dir(), snapshot]
	if not FileAccess.file_exists(snapshot_path):
		return {"ok": false, "reason": "missing_snapshot_screenshot", "path": snapshot_path}
	var image := Image.new()
	var load_err := image.load(snapshot_path)
	if load_err != OK:
		return {"ok": false, "reason": "snapshot_screenshot_load_failed", "path": snapshot_path, "actual": load_err}
	var sample_x := clampi(int(floor(overlap.get_center().x)), 0, image.get_width() - 1)
	var sample_y := clampi(int(floor(overlap.get_center().y)), 0, image.get_height() - 1)
	var pixel := image.get_pixel(sample_x, sample_y)
	var target_color := (target_item as ColorRect).color
	var other_color := (other_item as ColorRect).color
	var target_distance := _color_distance(pixel, target_color)
	var other_distance := _color_distance(pixel, other_color)
	if target_distance == other_distance:
		return {"ok": false, "reason": "ambiguous_snapshot_pixel_order", "path": snapshot_path, "point": [sample_x, sample_y]}
	return {
		"ok": true,
		"mode": "snapshot_pixel_overlap",
		"path": snapshot_path,
		"point": [sample_x, sample_y],
		"pixel": [pixel.r, pixel.g, pixel.b, pixel.a],
		"target_color_distance": target_distance,
		"other_color_distance": other_distance,
		"target_above_other": target_distance < other_distance,
	}


func _color_distance(a: Color, b: Color) -> float:
	return absf(a.r - b.r) + absf(a.g - b.g) + absf(a.b - b.b) + absf(a.a - b.a)


func _normalized_visual_rect_evidence(rect: Rect2, rect_source: String, coordinate_space: String = "viewport_2d") -> Dictionary:
	return {
		"ok": true,
		"rect": rect,
		"rect_source": rect_source,
		"coordinate_space": coordinate_space,
		"missing_evidence": "",
		"remediation": "",
	}


func _rect_from_visual_evidence(evidence: Dictionary) -> Dictionary:
	var raw_rect = evidence.get("rect", null)
	if not (raw_rect is Array):
		return {"ok": false}
	var values: Array = raw_rect
	if values.size() < 4:
		return {"ok": false}
	return {"ok": true, "rect": Rect2(float(values[0]), float(values[1]), float(values[2]), float(values[3]))}


func _visual_rect_remediation() -> String:
	return "provide qa_visual_rect as a Rect2, [x, y, width, height], or {rect, coordinate_space: \"viewport_2d\"} in rendered viewport pixels"


func _canvas_item_rect(item: CanvasItem) -> Dictionary:
	if item is Control:
		return _normalized_visual_rect_evidence((item as Control).get_global_rect(), "control_global_rect")
	if item is Line2D:
		return _line2d_rect(item as Line2D)
	return _normalized_visual_rect_evidence(Rect2(item.get_global_transform_with_canvas().origin - Vector2(2, 2), Vector2(4, 4)), "canvas_item_origin")


func _line2d_rect(line: Line2D) -> Dictionary:
	var points := line.points
	var base_evidence := {
		"ok": false,
		"rect_source": "line2d_points",
		"coordinate_space": "viewport_2d",
		"actual": {
			"class": line.get_class(),
			"point_count": points.size(),
			"width": float(line.width),
		},
		"remediation": _visual_rect_remediation(),
	}
	if points.size() < 2:
		base_evidence["missing_evidence"] = "nonzero_line2d_points"
		return base_evidence
	var transform := line.get_global_transform_with_canvas()
	var first := transform * points[0]
	var min_x := first.x
	var min_y := first.y
	var max_x := first.x
	var max_y := first.y
	var has_length := false
	var previous := first
	for index in range(1, points.size()):
		var current := transform * points[index]
		min_x = minf(min_x, current.x)
		min_y = minf(min_y, current.y)
		max_x = maxf(max_x, current.x)
		max_y = maxf(max_y, current.y)
		if previous.distance_to(current) > 0.001:
			has_length = true
		previous = current
	if not has_length:
		base_evidence["missing_evidence"] = "nonzero_line2d_length"
		base_evidence["actual"]["bounds"] = _rect_payload(Rect2(Vector2(min_x, min_y), Vector2(max_x - min_x, max_y - min_y)))
		return base_evidence
	var stroke_padding := maxf(float(line.width) * 0.5, 0.5) + 1.0
	var rect := Rect2(Vector2(min_x, min_y), Vector2(max_x - min_x, max_y - min_y)).grow(stroke_padding)
	if rect.size.x <= 0.0 or rect.size.y <= 0.0:
		base_evidence["missing_evidence"] = "measurable_line2d_rect"
		base_evidence["actual"]["bounds"] = _rect_payload(rect)
		return base_evidence
	return _normalized_visual_rect_evidence(rect, "line2d_points")


func _rect_payload(rect: Rect2) -> Array[float]:
	return [float(rect.position.x), float(rect.position.y), float(rect.size.x), float(rect.size.y)]


# Transform that maps 2D canvas coordinates (get_global_rect/qa_visual_rect) into
# the window/screenshot pixel space that visual captures are taken in. Under
# content scaling (project display/window/stretch/mode) this is a scale + offset;
# with no scaling it is the identity, so existing captures are unchanged.
func _canvas_to_screenshot_transform() -> Transform2D:
	if _root_window == null:
		return Transform2D.IDENTITY
	return _root_window.get_final_transform()


func _canvas_to_screenshot_payload(transform: Transform2D) -> Dictionary:
	var scale := transform.get_scale()
	return {
		"scale": [float(scale.x), float(scale.y)],
		"offset": [float(transform.origin.x), float(transform.origin.y)],
	}


func _canvas_visible_rect() -> Rect2:
	if _root_window == null:
		return Rect2()
	return _root_window.get_visible_rect()


# Loud guard against unforeseen capture/scaling mismatches: the full canvas rect,
# once mapped into screenshot space, must match the captured image size. If it
# does not, canvas-space rects cannot be indexed into this screenshot at all, so
# fail with a diagnostic error instead of silently sampling the wrong pixels.
# Returns {} when geometry is consistent, or a populated _owner.get_ref()._error() dict otherwise.
func _screenshot_geometry_mismatch_error(transform: Transform2D, image_size: Vector2, command_name: String, target: String, params: Dictionary, resolution: Dictionary, before_path: String, hidden_path: String) -> Dictionary:
	var canvas_rect := _canvas_visible_rect()
	var expected: Rect2 = transform * canvas_rect
	if absf(expected.size.x - image_size.x) <= 2.0 and absf(expected.size.y - image_size.y) <= 2.0:
		return {}
	return _owner.get_ref()._error(
		"screenshot_geometry_mismatch",
		"visual assertion screenshot size does not match the canvas-to-screenshot transform",
		{
			"command": command_name,
			"target": target,
			"snapshot": _visual_snapshot(params),
			"resolved": resolution.get("resolved", ""),
			"canvas_to_screenshot": _canvas_to_screenshot_payload(transform),
			"canvas_rect": _rect_payload(canvas_rect),
			"expected": {"screenshot_size": [expected.size.x, expected.size.y]},
			"actual": {"screenshot_size": [image_size.x, image_size.y]},
			"remediation": "the captured screenshot size must equal the content-scaled canvas render; ensure the visual capture is a real screenshot of the root window (not a fallback/synthetic image or a mismatched sub-viewport) taken at the active content-scale resolution",
			"artifacts": {"original": before_path, "target_hidden": hidden_path},
		},
	)


func _sanitize_visual_name(raw: String) -> String:
	return raw.replace(":", "_").replace("/", "_").replace("\\", "_")
