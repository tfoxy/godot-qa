extends RefCounted
## Exact recursive equality for JSON values read directly from Godot or JSON.
## JSON parsing represents numbers as floats, while providers often return ints.


static func equals(actual: Variant, expected: Variant) -> bool:
	var actual_type := typeof(actual)
	var expected_type := typeof(expected)
	if actual_type != expected_type:
		if actual_type == TYPE_INT and expected_type == TYPE_FLOAT:
			return _int_equals_float(actual, expected)
		if actual_type == TYPE_FLOAT and expected_type == TYPE_INT:
			return _int_equals_float(expected, actual)
		return false
	if actual is Array:
		if actual.size() != expected.size():
			return false
		for index in range(actual.size()):
			if not equals(actual[index], expected[index]):
				return false
		return true
	if actual is Dictionary:
		if actual.size() != expected.size():
			return false
		for key in actual:
			if not expected.has(key) or not equals(actual[key], expected[key]):
				return false
		return true
	return actual == expected


static func _int_equals_float(integer: int, number: float) -> bool:
	# The positive bound is exclusive: float(INT64_MAX) rounds to 2^63.
	# Check before converting so out-of-range values cannot saturate or wrap.
	if not is_finite(number) or number < -9223372036854775808.0 or number >= 9223372036854775808.0:
		return false
	if floor(number) != number:
		return false
	var converted := int(number)
	return converted == integer and float(converted) == number
