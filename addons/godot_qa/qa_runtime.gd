extends RefCounted

const QaRuntimeImpl = preload("res://addons/godot_qa/qa_runtime_impl.gd")

var _impl: RefCounted = null


func _init(scene_root: Node, root_window: Window, runtime_errors := [], held_actions := {}) -> void:
	_impl = QaRuntimeImpl.new(scene_root, root_window, runtime_errors, held_actions)


func execute(command_name: String, params := {}) -> Dictionary:
	return await _impl.execute(command_name, params)


func find_target_matches(target: String) -> Array:
	return _impl.find_target_matches(target)


func resolve_target(target: String) -> Dictionary:
	return _impl.resolve_target(target)


func resolve_snapshot_filter(params := {}) -> Dictionary:
	return _impl.resolve_snapshot_filter(params)


func resolve_point_provider(point_request: Variant, offset: Variant = null) -> Dictionary:
	return _impl.resolve_point_provider(point_request, offset)


func describe_probe_provider(provider_id: String) -> Dictionary:
	return _impl.describe_probe_provider(provider_id)


func read_state_provider(provider_id: String) -> Dictionary:
	return _impl.read_state_provider(provider_id)


func is_target_visible(node: Node) -> bool:
	return _impl.is_target_visible(node)


func viewport_rect() -> Rect2:
	return _impl.viewport_rect()


func ok(data := {}) -> Dictionary:
	return _impl.ok(data)


func error(error_type: String, message: String, data := {}) -> Dictionary:
	return _impl.error(error_type, message, data)


func process_frames(count: int) -> void:
	await _impl.process_frames(count)


static func register_qa_id(node: Node, qa_id: String, aliases: Variant = null) -> bool:
	return QaRuntimeImpl.register_qa_id(node, qa_id, aliases)


static func register_state_provider(provider_id: String, owner: Object, method_name: String, metadata: Dictionary = {}) -> Dictionary:
	return QaRuntimeImpl.register_state_provider(provider_id, owner, method_name, metadata)


static func unregister_state_provider(provider_id: String) -> Dictionary:
	return QaRuntimeImpl.unregister_state_provider(provider_id)


static func register_probe_provider(provider_id: String, owner: Object, method_name: String, metadata: Dictionary = {}) -> Dictionary:
	return QaRuntimeImpl.register_probe_provider(provider_id, owner, method_name, metadata)


static func unregister_probe_provider(provider_id: String) -> Dictionary:
	return QaRuntimeImpl.unregister_probe_provider(provider_id)


static func register_fixture_driver(driver_id: String, owner: Object, method_name: String, metadata: Dictionary = {}) -> Dictionary:
	return QaRuntimeImpl.register_fixture_driver(driver_id, owner, method_name, metadata)


static func unregister_fixture_driver(driver_id: String) -> Dictionary:
	return QaRuntimeImpl.unregister_fixture_driver(driver_id)


static func register_policy_provider(provider_id: String, owner: Object, method_name: String, metadata: Dictionary = {}) -> Dictionary:
	return QaRuntimeImpl.register_policy_provider(provider_id, owner, method_name, metadata)


static func unregister_policy_provider(provider_id: String) -> Dictionary:
	return QaRuntimeImpl.unregister_policy_provider(provider_id)


static func clear_provider_registry() -> void:
	QaRuntimeImpl.clear_provider_registry()


static func list_state_providers() -> Array[Dictionary]:
	return QaRuntimeImpl.list_state_providers()


static func list_probe_providers() -> Array[Dictionary]:
	return QaRuntimeImpl.list_probe_providers()


static func list_fixture_drivers() -> Array[Dictionary]:
	return QaRuntimeImpl.list_fixture_drivers()


static func list_policy_providers() -> Array[Dictionary]:
	return QaRuntimeImpl.list_policy_providers()


# QA-aware replacement for Input.get_connected_joypads(): the sorted, deduped
# union of the real engine roster and joypads faked via the joy_connect step.
# With no fakes it returns exactly the engine roster, so games can adopt it
# unconditionally (same one-line opt-in story as QaArgs.effective_user_args()).
static func effective_connected_joypads() -> Array[int]:
	return QaRuntimeImpl.effective_connected_joypads()


func describe_node(node: Variant) -> String:
	return _impl.describe_node(node)


func stable_locator_for_node(node: Node) -> String:
	return _impl.stable_locator_for_node(node)


func release_held_actions() -> Array[Dictionary]:
	return _impl.release_held_actions()


func release_held_axes() -> Array[Dictionary]:
	return await _impl.release_held_axes()


func release_held_buttons() -> Array[Dictionary]:
	return await _impl.release_held_buttons()


func release_held_touches() -> Array[Dictionary]:
	return await _impl.release_held_touches()


func release_faked_joypads() -> Array[Dictionary]:
	return await _impl.release_faked_joypads()


func reset_input() -> Array[Dictionary]:
	return await _impl.reset_input()


func teardown() -> void:
	_impl.teardown()
