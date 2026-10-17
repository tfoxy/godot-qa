extends Node

const SAMPLE_RATE := 44100
const CHANNELS := 2
const BITS_PER_SAMPLE := 16
const TRACK_SECONDS := 4.0
const DEFAULT_TRACK := "deterministic_transients"
const TONE_TRACK := "deterministic_tone_mix"

@onready var player: AudioStreamPlayer = $AudioStreamPlayer

var _pcm_cache := {}
var _stream_cache := {}


func _ready() -> void:
	set_meta("qa_id", "audio_controller")
	add_to_group("qa.id.audio_controller")
	add_to_group("qa.audio_controller")
	_build_track(DEFAULT_TRACK)


func _exit_tree() -> void:
	if player != null:
		player.stop()
		player.stream = null
	_stream_cache.clear()


func qa_export_audio_track(track_name: String = DEFAULT_TRACK, context: Dictionary = {}) -> PackedByteArray:
	if not _is_known_track(track_name):
		return PackedByteArray()
	var pcm: PackedByteArray = _build_track(track_name)
	var wav_bytes := _wav_from_pcm(pcm)
	var max_bytes := int(context.get("max_bytes", 0))
	if max_bytes > 0 and wav_bytes.size() > max_bytes:
		return PackedByteArray()
	return wav_bytes


func qa_start_deterministic_playback(track_name: String = DEFAULT_TRACK) -> Dictionary:
	if not _is_known_track(track_name):
		return {
			"ok": false,
			"track": track_name,
			"expected": [DEFAULT_TRACK, TONE_TRACK],
		}
	var pcm: PackedByteArray = _build_track(track_name)
	if not _stream_cache.has(track_name):
		var stream := AudioStreamWAV.new()
		stream.format = AudioStreamWAV.FORMAT_16_BITS
		stream.mix_rate = SAMPLE_RATE
		stream.stereo = true
		stream.loop_mode = AudioStreamWAV.LOOP_DISABLED
		stream.data = pcm
		_stream_cache[track_name] = stream
	player.bus = "Master"
	player.stream = _stream_cache[track_name]
	player.play()
	return {
		"ok": true,
		"track": track_name,
		"bus": player.bus,
		"duration_seconds": TRACK_SECONDS,
		"sample_rate": SAMPLE_RATE,
		"host_microphone_capture": false,
	}


func qa_stop_deterministic_playback() -> Dictionary:
	player.stop()
	player.stream = null
	return {"ok": true, "stopped": true}


func qa_audio_fixture_state() -> Dictionary:
	return {
		"qa_id": "audio_controller",
		"tracks": [DEFAULT_TRACK, TONE_TRACK],
		"default_track": DEFAULT_TRACK,
		"bus": "Master",
		"sample_rate": SAMPLE_RATE,
		"duration_seconds": TRACK_SECONDS,
	}


func _is_known_track(track_name: String) -> bool:
	return track_name == DEFAULT_TRACK or track_name == TONE_TRACK


func _build_track(track_name: String) -> PackedByteArray:
	if _pcm_cache.has(track_name):
		return _pcm_cache[track_name]
	var pcm := PackedByteArray()
	if track_name == TONE_TRACK:
		pcm = _generate_tone_mix_pcm()
	else:
		pcm = _generate_transient_fixture_pcm()
	_pcm_cache[track_name] = pcm
	return pcm


func _generate_transient_fixture_pcm() -> PackedByteArray:
	var bytes := PackedByteArray()
	var sample_count := int(TRACK_SECONDS * SAMPLE_RATE)
	bytes.resize(sample_count * CHANNELS * 2)
	var write_offset := 0
	for i in sample_count:
		var t := float(i) / float(SAMPLE_RATE)
		var mono := 0.0
		if t < 1.0:
			mono = 0.0
		elif t < 2.0:
			var tone_t := t - 1.0
			mono = sin(TAU * 440.0 * tone_t) * 0.36
		elif t < 3.0:
			var local_index := i - int(2.0 * SAMPLE_RATE)
			var burst_period := int(SAMPLE_RATE / 24)
			var burst_index := local_index % burst_period
			if burst_index < 72:
				var burst_env := exp(-float(burst_index) / 13.0)
				mono = sin(TAU * 1800.0 * t) * burst_env * 0.82
			else:
				mono = sin(TAU * 220.0 * t) * 0.04
		else:
			var clip_t := t - 3.0
			var square := 1.0
			if sin(TAU * 110.0 * clip_t) < 0.0:
				square = -1.0
			mono = clamp(square * 1.04 + sin(TAU * 660.0 * clip_t) * 0.12, -1.0, 1.0)
		var pan := sin(TAU * 0.25 * t) * 0.12
		var left := int(clamp(mono * (1.0 - pan), -1.0, 1.0) * 32767.0)
		var right := int(clamp(mono * (1.0 + pan), -1.0, 1.0) * 32767.0)
		bytes.encode_s16(write_offset, left)
		bytes.encode_s16(write_offset + 2, right)
		write_offset += 4
	return bytes


func _generate_tone_mix_pcm() -> PackedByteArray:
	var bytes := PackedByteArray()
	var sample_count := int(TRACK_SECONDS * SAMPLE_RATE)
	bytes.resize(sample_count * CHANNELS * 2)
	var write_offset := 0
	for i in sample_count:
		var t := float(i) / float(SAMPLE_RATE)
		var envelope: float = min(1.0, t * 4.0) * min(1.0, (TRACK_SECONDS - t) * 4.0)
		var low: float = sin(TAU * 110.0 * t) * 0.22
		var mid: float = sin(TAU * 440.0 * t) * 0.30
		var high: float = sin(TAU * 1760.0 * t) * 0.10
		var mono: float = (low + mid + high) * envelope
		var left := int(clamp(mono * 0.94, -0.95, 0.95) * 32767.0)
		var right := int(clamp(mono * 1.06, -0.95, 0.95) * 32767.0)
		bytes.encode_s16(write_offset, left)
		bytes.encode_s16(write_offset + 2, right)
		write_offset += 4
	return bytes


func _wav_from_pcm(pcm: PackedByteArray) -> PackedByteArray:
	var wav := PackedByteArray()
	var data_size := pcm.size()
	var byte_rate := SAMPLE_RATE * CHANNELS * BITS_PER_SAMPLE / 8
	var block_align := CHANNELS * BITS_PER_SAMPLE / 8
	wav.resize(44 + data_size)
	_write_ascii(wav, 0, "RIFF")
	wav.encode_u32(4, 36 + data_size)
	_write_ascii(wav, 8, "WAVE")
	_write_ascii(wav, 12, "fmt ")
	wav.encode_u32(16, 16)
	wav.encode_u16(20, 1)
	wav.encode_u16(22, CHANNELS)
	wav.encode_u32(24, SAMPLE_RATE)
	wav.encode_u32(28, byte_rate)
	wav.encode_u16(32, block_align)
	wav.encode_u16(34, BITS_PER_SAMPLE)
	_write_ascii(wav, 36, "data")
	wav.encode_u32(40, data_size)
	for i in data_size:
		wav[44 + i] = pcm[i]
	return wav


func _write_ascii(bytes: PackedByteArray, offset: int, text: String) -> void:
	var ascii := text.to_ascii_buffer()
	for i in ascii.size():
		bytes[offset + i] = ascii[i]
