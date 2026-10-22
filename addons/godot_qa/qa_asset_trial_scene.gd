extends RefCounted

# SceneState checks cover authored topology, instance boundaries, connections,
# ownership and all authored values. Resource IDs and property ordering are not
# semantic; embedded storage is recursively compared instead.
static func value(value: Variant, seen: Dictionary = {}) -> Variant:
	if value is Resource:
		var resource := value as Resource
		var path := resource.resource_path
		if path.begins_with("res://") and not path.contains("::"):
			return {"external": path, "class": resource.get_class(), "sha256": FileAccess.get_sha256(path)}
		if seen.has(resource.get_instance_id()):
			return {"cycle": resource.get_class()}
		var nested := seen.duplicate()
		nested[resource.get_instance_id()] = true
		var properties := {}
		for property in resource.get_property_list():
			var name := str(property.name)
			if int(property.usage) & PROPERTY_USAGE_STORAGE and name not in ["resource_path", "resource_name", "resource_local_to_scene"]:
				properties[name] = value(resource.get(name), nested)
		properties["resource_name"] = resource.resource_name
		properties["resource_local_to_scene"] = resource.resource_local_to_scene
		return {"embedded": resource.get_class(), "properties": properties}
	if value is Array:
		var array: Array = []
		for item in value:
			array.append(value(item, seen))
		return array
	if value is Dictionary:
		var dictionary := {}
		for key in value:
			dictionary[var_to_str(key)] = value(value[key], seen)
		return dictionary
	# var_to_str retains type distinctions, native vectors and packed arrays.
	return var_to_str(value)


static func state(packed: PackedScene, excluded: Dictionary) -> Dictionary:
	var scene := packed.get_state()
	var nodes := {}
	var resource_sites := {}
	for index in scene.get_node_count():
		var path := str(scene.get_node_path(index)).trim_prefix("./")
		var properties := {}
		for property_index in scene.get_node_property_count(index):
			var name := str(scene.get_node_property_name(index, property_index))
			if not excluded.has(path + ":" + name):
				var stored = scene.get_node_property_value(index, property_index)
				properties[name] = value(stored)
				alias_sites(stored, path + ":" + name, resource_sites)
		var node_type := str(scene.get_node_type(index))
		var instance := scene.get_node_instance(index)
		if node_type == "" and instance != null:
			var instance_state := instance.get_state()
			while instance_state != null and node_type == "":
				node_type = str(instance_state.get_node_type(0))
				instance_state = instance_state.get_base_scene_state()
		nodes[path] = {
			"type": node_type, "name": str(scene.get_node_name(index)),
			"owner": str(scene.get_node_owner_path(index)), "index": scene.get_node_index(index),
			"instance": value(scene.get_node_instance(index)), "placeholder": scene.get_node_instance_placeholder(index),
			"groups": Array(scene.get_node_groups(index)), "properties": properties,
		}
	var connections: Array = []
	for index in scene.get_connection_count():
		connections.append({
			"source": str(scene.get_connection_source(index)), "signal": str(scene.get_connection_signal(index)),
			"target": str(scene.get_connection_target(index)), "method": str(scene.get_connection_method(index)),
			"flags": scene.get_connection_flags(index), "binds": value(scene.get_connection_binds(index)),
			"unbinds": scene.get_connection_unbinds(index),
		})
	connections.sort_custom(func(a, b): return JSON.stringify(a) < JSON.stringify(b))
	var aliases: Array = []
	for sites in resource_sites.values():
		if sites.size() > 1:
			sites.sort()
			aliases.append(sites)
	aliases.sort_custom(func(a, b): return JSON.stringify(a) < JSON.stringify(b))
	return {"nodes": nodes, "connections": connections, "resource_aliases": aliases, "base_scene": value(scene.get_base_scene_state()) if scene.get_base_scene_state() != null else null}


static func alias_sites(stored: Variant, site: String, sites: Dictionary, ancestors: Dictionary = {}) -> void:
	if stored is Resource:
		var resource := stored as Resource
		if resource.resource_path.begins_with("res://") and not resource.resource_path.contains("::"):
			return
		var id := resource.get_instance_id()
		if not sites.has(id):
			sites[id] = []
		sites[id].append(site)
		if ancestors.has(id):
			return
		var nested := ancestors.duplicate()
		nested[id] = true
		for property in resource.get_property_list():
			if int(property.usage) & PROPERTY_USAGE_STORAGE:
				alias_sites(resource.get(str(property.name)), site + "." + str(property.name), sites, nested)
	elif stored is Array:
		for index in stored.size():
			alias_sites(stored[index], site + "[" + str(index) + "]", sites, ancestors)
	elif stored is Dictionary:
		for key in stored:
			alias_sites(stored[key], site + "[" + var_to_str(key) + "]", sites, ancestors)


static func uid(path: String) -> String:
	var file := FileAccess.open(path, FileAccess.READ)
	if file == null:
		return ""
	var header := file.get_line()
	var regex := RegEx.new()
	regex.compile('uid="(uid://[^"\\s]+)"')
	var found := regex.search(header)
	return found.get_string(1) if found != null else ""


static func compare(original: Dictionary, candidate: Dictionary, source_root: Node) -> Dictionary:
	if original.get("resource_aliases") != candidate.get("resource_aliases"):
		return {"ok": false, "target": "resource_aliases", "expected": original.get("resource_aliases"), "actual": candidate.get("resource_aliases")}
	if original.get("connections") != candidate.get("connections"):
		return {"ok": false, "target": "connections", "expected": original.get("connections"), "actual": candidate.get("connections")}
	var before: Dictionary = original.get("nodes", {})
	var after: Dictionary = candidate.get("nodes", {})
	if before.keys() != after.keys():
		return {"ok": false, "target": "topology", "expected": before.keys(), "actual": after.keys()}
	for path in before:
		var left: Dictionary = before[path].duplicate(true)
		var right: Dictionary = after[path].duplicate(true)
		var left_properties: Dictionary = left.get("properties", {})
		var right_properties: Dictionary = right.get("properties", {})
		# Packing can elide authored defaults or add native defaults. Permit only
		# values proven identical on a pristine script-free native node, never
		# values supplied by _init or custom setters.
		var node := source_root.get_node_or_null(NodePath(path))
		var native: Object = ClassDB.instantiate(node.get_class()) if node != null and ClassDB.can_instantiate(node.get_class()) else null
		for name in left_properties.keys():
			if not right_properties.has(name) and native != null and value(native.get(name)) == left_properties[name]:
				left_properties.erase(name)
		for name in right_properties.keys():
			if not left_properties.has(name) and native != null and value(native.get(name)) == right_properties[name]:
				right_properties.erase(name)
		if native != null:
			native.free()
		if left != right:
			return {"ok": false, "target": path, "expected": left, "actual": right}
	return {"ok": true}
