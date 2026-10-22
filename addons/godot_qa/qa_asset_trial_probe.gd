extends RefCounted

# Private asset-trial facts. No bridge command or generic resource codec.
static func backend() -> Dictionary:
	return {
		"engine_version": str(Engine.get_version_info().get("string", "")),
		"display_server": DisplayServer.get_name(),
		"rendering_method": RenderingServer.get_current_rendering_method(),
		"rendering_driver": RenderingServer.get_current_rendering_driver_name(),
		"video_adapter_name": RenderingServer.get_video_adapter_name(),
		"video_adapter_vendor": RenderingServer.get_video_adapter_vendor(),
		"video_adapter_type": RenderingServer.get_video_adapter_type(),
		"audio_driver": AudioServer.get_driver_name(),
	}


static func digest(bytes: PackedByteArray) -> String:
	var context := HashingContext.new()
	context.start(HashingContext.HASH_SHA256)
	context.update(bytes)
	return context.finish().hex_encode()


static func resource_facts(resource: Resource, fingerprint: bool = true) -> Dictionary:
	if resource == null:
		return {"null": true}
	var path := resource.resource_path
	var uid := ResourceLoader.get_resource_uid(path) if path.begins_with("res://") and not path.contains("::") else -1
	var result := {
		"null": false, "path": path, "class": resource.get_class(),
		"uid": ResourceUID.id_to_text(uid) if uid >= 0 else "",
		"source_sha256": FileAccess.get_sha256(path) if fingerprint and FileAccess.file_exists(path) else "",
		"instance_id": resource.get_instance_id(),
	}
	if not fingerprint:
		return result
	if resource is Texture2D:
		var image := (resource as Texture2D).get_image()
		if image == null or image.is_empty():
			result["fingerprint_error"] = "asset_fingerprint_unavailable"
		else:
			image.convert(Image.FORMAT_RGBA8)
			result["fingerprint"] = {"kind": "rgba8", "width": image.get_width(), "height": image.get_height(), "sha256": digest(image.get_data())}
	elif resource is AudioStreamWAV:
		var stream := resource as AudioStreamWAV
		result["fingerprint"] = {
			"kind": "wav", "format": stream.format, "mix_rate": stream.mix_rate,
			"stereo": stream.stereo, "loop_mode": stream.loop_mode,
			"loop_begin": stream.loop_begin, "loop_end": stream.loop_end,
			"sha256": digest(stream.data),
		}
	else:
		result["fingerprint_error"] = "asset_fingerprint_unavailable"
	return result


static func slot_supported(node: Node, property: String) -> bool:
	return (property == "texture" and (node is Sprite2D or node is TextureRect)) or (property == "stream" and (node is AudioStreamPlayer or node is AudioStreamPlayer2D))


static func binding_facts(runtime: RefCounted, binding: Dictionary, fingerprint: bool = true) -> Dictionary:
	var target := str(binding.get("runtime_target", ""))
	var property := str(binding.get("property", ""))
	var result := {"id": str(binding.get("id", "")), "target": target, "property": property}
	var resolution: Dictionary = runtime.resolve_target(target)
	if not bool(resolution.get("ok", false)):
		result["error"] = resolution.get("error", {})
		return result
	var node := resolution.get("node") as Node
	if node == null or not slot_supported(node, property):
		result["error"] = {"type": "asset_binding_unconfirmed", "message": "target does not have a supported native slot"}
		return result
	result.merge({
		"node_path": str(node.get_path()), "node_class": node.get_class(),
		"instance_id": node.get_instance_id(), "owner_path": str(node.owner.get_path()) if node.owner != null else "",
		"resource": resource_facts(node.get(property) as Resource, fingerprint),
	})
	if property == "stream":
		result["bus"] = str(node.get("bus"))
		result["playing"] = bool(node.get("playing"))
		result["stream_paused"] = bool(node.get("stream_paused"))
	return result
