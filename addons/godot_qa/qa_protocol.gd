extends RefCounted

const PROTOCOL_VERSION := "0.1"

static func ok(data := {}, request_id := "") -> Dictionary:
	var response := {
		"ok": true,
		"protocol_version": PROTOCOL_VERSION,
		"data": data if data is Dictionary else {},
	}
	if request_id != "":
		response["id"] = request_id
	return response

static func error(error_type: String, message: String, data := {}, request_id := "") -> Dictionary:
	var response := {
		"ok": false,
		"protocol_version": PROTOCOL_VERSION,
		"error": {
			"type": error_type,
			"message": message,
			"data": data if data is Dictionary else {},
		},
	}
	if request_id != "":
		response["id"] = request_id
	return response
