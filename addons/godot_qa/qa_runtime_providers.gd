extends RefCounted

var _runtime: WeakRef


func _init(runtime: RefCounted) -> void:
	_runtime = weakref(runtime)


const PROVIDER_ID_ALLOWED_EXTRA := "_.:-"
const PROVIDER_MUTATING_PREFIXES := ["set_", "add_", "remove_", "delete_", "clear_", "queue_", "free", "call_", "emit_", "change_", "qa_mutate_"]


static func _register_provider(registry: Dictionary, provider_kind: String, provider_id: String, owner: Object, method_name: String, metadata: Dictionary, argument_count: int, reject_mutating_name: bool) -> Dictionary:
	var normalized_id := str(provider_id).strip_edges()
	var normalized_method := str(method_name).strip_edges()
	if normalized_id == "" or not _provider_id_is_valid(normalized_id):
		return _static_error("invalid_provider_id", "Provider id must be non-empty and use only ASCII letters, numbers, '.', '_', ':', or '-'", {"provider_kind": provider_kind, "provider_id": provider_id})
	if registry.has(normalized_id):
		return _static_error("provider_already_registered", "Provider id is already registered", {"provider_kind": provider_kind, "provider_id": normalized_id})
	if owner == null or not is_instance_valid(owner):
		return _static_error("provider_owner_invalid", "Provider owner must be a valid Object", {"provider_kind": provider_kind, "provider_id": normalized_id})
	if normalized_method == "" or not owner.has_method(normalized_method):
		return _static_error("provider_method_missing", "Provider method was not found on the owner", {"provider_kind": provider_kind, "provider_id": normalized_id, "method": normalized_method})
	if reject_mutating_name and _provider_method_name_is_mutating(normalized_method):
		return _static_error("provider_method_mutating", "Read-only providers cannot register mutating method names", {"provider_kind": provider_kind, "provider_id": normalized_id, "method": normalized_method})
	var reflected_count := _registered_method_argument_count(owner, normalized_method)
	if reflected_count != argument_count:
		return _static_error("provider_method_arguments", "Provider method has an unsupported argument count", {"provider_kind": provider_kind, "provider_id": normalized_id, "method": normalized_method, "expected": argument_count, "actual": reflected_count})
	if not _provider_registry_json_compatible(metadata):
		return _static_error("provider_metadata_invalid", "Provider metadata must be JSON-compatible", {"provider_kind": provider_kind, "provider_id": normalized_id})
	registry[normalized_id] = {
		"id": normalized_id,
		"kind": provider_kind,
		"owner": owner,
		"method": normalized_method,
		"metadata": metadata.duplicate(true),
	}
	return {"ok": true, "data": _provider_public_entry(registry[normalized_id])}


static func _unregister_provider(registry: Dictionary, provider_kind: String, provider_id: String) -> Dictionary:
	var normalized_id := str(provider_id).strip_edges()
	if normalized_id == "" or not registry.has(normalized_id):
		return _static_error("provider_not_registered", "Provider id is not registered", {"provider_kind": provider_kind, "provider_id": normalized_id})
	var entry := _provider_public_entry(registry.get(normalized_id, {}))
	registry.erase(normalized_id)
	return {"ok": true, "data": entry}


static func _list_providers(registry: Dictionary) -> Array[Dictionary]:
	var ids: Array[String] = []
	for provider_id in registry.keys():
		ids.append(str(provider_id))
	ids.sort()
	var providers: Array[Dictionary] = []
	for provider_id in ids:
		var entry = registry.get(provider_id, {})
		if entry is Dictionary:
			providers.append(_provider_public_entry(entry))
	return providers


static func _provider_public_entry(entry: Dictionary) -> Dictionary:
	var owner = entry.get("owner", null)
	var owner_path := ""
	var owner_class := ""
	if owner is Node:
		var owner_node := owner as Node
		owner_path = str(owner_node.get_path()) if owner_node.is_inside_tree() else ""
	if owner is Object:
		owner_class = (owner as Object).get_class()
	return {
		"id": str(entry.get("id", "")),
		"kind": str(entry.get("kind", "")),
		"method": str(entry.get("method", "")),
		"owner_class": owner_class,
		"owner_path": owner_path,
		"metadata": (entry.get("metadata", {}) as Dictionary).duplicate(true) if entry.get("metadata", {}) is Dictionary else {},
	}


static func _provider_id_is_valid(provider_id: String) -> bool:
	for character in provider_id:
		var codepoint := character.unicode_at(0)
		var is_digit := codepoint >= 48 and codepoint <= 57
		var is_upper := codepoint >= 65 and codepoint <= 90
		var is_lower := codepoint >= 97 and codepoint <= 122
		if not (is_digit or is_upper or is_lower or PROVIDER_ID_ALLOWED_EXTRA.contains(character)):
			return false
	return true


static func _provider_method_name_is_mutating(method_name: String) -> bool:
	var normalized := method_name.strip_edges()
	while normalized.begins_with("_"):
		normalized = normalized.substr(1)
	for prefix in PROVIDER_MUTATING_PREFIXES:
		if normalized.begins_with(prefix):
			return true
	return false


static func _registered_method_argument_count(owner: Object, method_name: String) -> int:
	for method_info in owner.get_method_list():
		if not method_info is Dictionary:
			continue
		var info: Dictionary = method_info
		if str(info.get("name", "")) != method_name:
			continue
		var args: Array = info.get("args", []) if info.get("args", []) is Array else []
		return args.size()
	return -1


static func _provider_registry_json_compatible(value: Variant) -> bool:
	match typeof(value):
		TYPE_NIL, TYPE_BOOL, TYPE_INT, TYPE_FLOAT, TYPE_STRING:
			return true
		TYPE_STRING_NAME, TYPE_NODE_PATH:
			return true
		TYPE_ARRAY:
			for item in value:
				if not _provider_registry_json_compatible(item):
					return false
			return true
		TYPE_DICTIONARY:
			var dictionary: Dictionary = value
			for key in dictionary.keys():
				if not _provider_registry_json_compatible(dictionary[key]):
					return false
			return true
		_:
			return false


static func _static_error(error_type: String, message: String, data: Dictionary) -> Dictionary:
	return {
		"ok": false,
		"error": {
			"type": error_type,
			"message": message,
			"data": data,
		},
	}


func _provider_entry(registry: Dictionary, provider_id: String, provider_kind: String) -> Dictionary:
	var normalized_id := str(provider_id).strip_edges()
	if normalized_id == "":
		return _runtime.get_ref()._error("provider_missing", "Provider id is required", {"provider_kind": provider_kind, "provider": provider_id})
	if not registry.has(normalized_id):
		return _runtime.get_ref()._error("provider_not_registered", "Provider id is not registered", {"provider_kind": provider_kind, "provider": normalized_id})
	var entry = registry.get(normalized_id, {})
	if not (entry is Dictionary):
		return _runtime.get_ref()._error("provider_invalid", "Provider registry entry is invalid", {"provider_kind": provider_kind, "provider": normalized_id})
	var provider_entry: Dictionary = entry
	var owner = provider_entry.get("owner", null)
	if owner == null or not is_instance_valid(owner):
		registry.erase(normalized_id)
		return _runtime.get_ref()._error("provider_owner_invalid", "Provider owner is no longer valid", {"provider_kind": provider_kind, "provider": normalized_id})
	var method_name := str(provider_entry.get("method", ""))
	if method_name == "" or not (owner as Object).has_method(method_name):
		return _runtime.get_ref()._error("provider_method_missing", "Provider method is no longer available", {"provider_kind": provider_kind, "provider": normalized_id, "method": method_name})
	return {"ok": true, "entry": provider_entry}


func read_state_provider(provider_id: String) -> Dictionary:
	var provider := _provider_entry(_runtime.get_ref()._state_providers, provider_id, "state_provider")
	if not bool(provider.get("ok", false)):
		return provider
	var entry: Dictionary = provider.get("entry", {})
	var owner: Object = entry.get("owner")
	var method_name := str(entry.get("method", ""))
	var value = owner.call(method_name)
	if not _runtime.get_ref()._is_json_compatible_project_state_value(value):
		return _runtime.get_ref()._error(
			"invalid_state_source",
			"registered state provider returned non JSON-compatible data",
			{
				"provider": str(entry.get("id", "")),
				"method": method_name,
				"expected": "JSON-compatible Dictionary or Array",
				"actual_type": type_string(typeof(value)),
			},
		)
	if not (value is Dictionary or value is Array):
		return _runtime.get_ref()._error(
			"invalid_state_source",
			"registered state provider must return a Dictionary or Array",
			{
				"provider": str(entry.get("id", "")),
				"method": method_name,
				"expected": "Dictionary or Array",
				"actual": value,
				"actual_type": type_string(typeof(value)),
			},
		)
	return {
		"ok": true,
		"provider": str(entry.get("id", "")),
		"method": method_name,
		"metadata": (entry.get("metadata", {}) as Dictionary).duplicate(true) if entry.get("metadata", {}) is Dictionary else {},
		"state": value,
	}


func call_probe_provider(provider_id: String, request: Dictionary) -> Dictionary:
	var provider := _provider_entry(_runtime.get_ref()._probe_providers, provider_id, "probe_provider")
	if not bool(provider.get("ok", false)):
		return provider
	var entry: Dictionary = provider.get("entry", {})
	var owner: Object = entry.get("owner")
	var method_name := str(entry.get("method", ""))
	var request_payload := request.duplicate(true)
	request_payload["provider_id"] = str(entry.get("id", ""))
	var result = owner.call(method_name, request_payload)
	if not _runtime.get_ref()._is_json_compatible_project_state_value(result):
		return _runtime.get_ref()._error(
			"invalid_probe_result",
			"registered probe provider returned non JSON-compatible data",
			{
				"provider_id": str(entry.get("id", "")),
				"method": method_name,
				"expected": "JSON-compatible Dictionary",
				"actual_type": type_string(typeof(result)),
			},
		)
	if not (result is Dictionary):
		return _runtime.get_ref()._error(
			"invalid_probe_result",
			"registered probe provider must return a Dictionary",
			{
				"provider_id": str(entry.get("id", "")),
				"method": method_name,
				"expected": "Dictionary",
				"actual": result,
			},
		)
	var result_dict: Dictionary = result
	result_dict["provider_id"] = str(entry.get("id", ""))
	if entry.get("metadata", {}) is Dictionary and (entry.get("metadata", {}) as Dictionary).has("provider_method"):
		result_dict["provider_method"] = str((entry.get("metadata", {}) as Dictionary).get("provider_method", ""))
	return {"ok": true, "provider_id": str(entry.get("id", "")), "method": method_name, "result": result_dict}


func describe_probe_provider(provider_id: String) -> Dictionary:
	var provider := _provider_entry(_runtime.get_ref()._probe_providers, provider_id, "probe_provider")
	if not bool(provider.get("ok", false)):
		return provider
	var entry: Dictionary = provider.get("entry", {})
	return {"ok": true, "data": _provider_public_entry(entry)}
