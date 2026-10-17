extends Node

const SAMPLE_RATE := 44100
const CHANNELS := 2
const BITS_PER_SAMPLE := 16
const LOOP_SECONDS := 4.0
const LOOP_NAME := "dodgeball_rush_loop"
const PEAK_AMPLITUDE := 0.42

@onready var player: AudioStreamPlayer = $AudioStreamPlayer

var _pcm_cache := PackedByteArray()
var _stream_cache: AudioStreamWAV
var _render_phase := 0


func _ready() -> void:
	set_meta("qa_id", "audio_controller")
	add_to_group("qa.id.audio_controller")
	add_to_group("qa.audio_controller")
	_build_audio_cache()


func _exit_tree() -> void:
	if player != null:
		player.stop()
		player.stream = null
	_stream_cache = null


func qa_export_audio_track(track_name: String = LOOP_NAME, context: Dictionary = {}) -> PackedByteArray:
	if track_name != LOOP_NAME:
		return PackedByteArray()
	var wav_bytes := _wav_from_pcm(_pcm_cache)
	var max_bytes := int(context.get("max_bytes", 0))
	if max_bytes > 0 and wav_bytes.size() > max_bytes:
		return PackedByteArray()
	return wav_bytes


func export_audio_track(track_name: String = LOOP_NAME, context: Dictionary = {}) -> PackedByteArray:
	return qa_export_audio_track(track_name, context)


## Voice-fill fixture for `godot-qa audio render`: the tool owns the bake loop length and the
## WAV container; this only synthesizes `frame_count` phase-continuous mono samples in [-1, 1].
func qa_fill_audio(frame_count: int, mix_rate: int, _track_name: String = LOOP_NAME) -> PackedFloat32Array:
	var samples := PackedFloat32Array()
	samples.resize(maxi(0, frame_count))
	for i in frame_count:
		var t: float = float(_render_phase + i) / float(maxi(1, mix_rate))
		var beat_phase: float = fmod(t * 2.0, 1.0)
		var beat_env: float = exp(-beat_phase * 7.0)
		var bass: float = sin(TAU * 110.0 * t) * (0.45 + 0.35 * beat_env)
		var lead: float = sin(TAU * 330.0 * t) * 0.24
		var hat: float = sin(TAU * 3520.0 * t) * exp(-beat_phase * 22.0) * 0.08
		samples[i] = clampf((bass + lead + hat) * PEAK_AMPLITUDE, -0.95, 0.95)
	_render_phase += frame_count
	return samples


func qa_export_oversized_audio(_track_name: String = LOOP_NAME, _context: Dictionary = {}) -> PackedByteArray:
	var bytes := PackedByteArray()
	bytes.resize(10485761)
	_write_ascii(bytes, 0, "RIFF")
	_write_ascii(bytes, 8, "WAVE")
	return bytes


func qa_export_silent_audio(_track_name: String = LOOP_NAME, _context: Dictionary = {}) -> PackedByteArray:
	var sample_count := int(1.0 * SAMPLE_RATE)
	var pcm := PackedByteArray()
	pcm.resize(sample_count * CHANNELS * 2)
	return _wav_from_pcm(pcm)


func qa_start_deterministic_playback(track_name: String = LOOP_NAME) -> Dictionary:
	if track_name != LOOP_NAME:
		return {
			"ok": false,
			"track": track_name,
			"expected": LOOP_NAME,
		}
	_build_audio_cache()
	player.bus = "Master"
	player.stream = _stream_cache
	player.play()
	return {
		"ok": true,
		"track": LOOP_NAME,
		"bus": player.bus,
		"duration_seconds": LOOP_SECONDS,
		"sample_rate": SAMPLE_RATE,
	}


func qa_stop_deterministic_playback() -> Dictionary:
	player.stop()
	player.stream = null
	return {"ok": true, "stopped": true}


func qa_export_rejected_path(_track_name: String = LOOP_NAME, context: Dictionary = {}) -> Dictionary:
	return {
		"wav": "res://outside-controlled-audio-dir.wav",
		"audio_dir": str(context.get("audio_dir", "")),
	}


func qa_export_non_json_metadata(_track_name: String = LOOP_NAME, context: Dictionary = {}) -> Dictionary:
	return {
		"wav": str(context.get("wav_path", "")),
		"metadata": self,
	}


func _build_audio_cache() -> void:
	if not _pcm_cache.is_empty() and _stream_cache != null:
		return
	_pcm_cache = _generate_loop_pcm()
	_stream_cache = AudioStreamWAV.new()
	_stream_cache.format = AudioStreamWAV.FORMAT_16_BITS
	_stream_cache.mix_rate = SAMPLE_RATE
	_stream_cache.stereo = true
	_stream_cache.loop_mode = AudioStreamWAV.LOOP_FORWARD
	_stream_cache.loop_begin = 0
	_stream_cache.loop_end = int(LOOP_SECONDS * SAMPLE_RATE)
	_stream_cache.data = _pcm_cache


func _generate_loop_pcm() -> PackedByteArray:
	var bytes := PackedByteArray()
	var sample_count := int(LOOP_SECONDS * SAMPLE_RATE)
	bytes.resize(sample_count * CHANNELS * 2)
	var write_offset := 0
	for i in sample_count:
		var t: float = float(i) / float(SAMPLE_RATE)
		var beat_phase: float = fmod(t * 2.0, 1.0)
		var beat_env: float = exp(-beat_phase * 7.0)
		var bar_phase: float = fmod(t / LOOP_SECONDS, 1.0)
		var bass: float = sin(TAU * 110.0 * t) * (0.45 + 0.35 * beat_env)
		var lead: float = sin(TAU * (330.0 + 55.0 * sin(TAU * bar_phase)) * t) * 0.24
		var pulse: float = sin(TAU * 880.0 * t) * beat_env * 0.16
		var hat: float = sin(TAU * 3520.0 * t) * exp(-beat_phase * 22.0) * 0.08
		var mono: float = clamp((bass + lead + pulse + hat) * PEAK_AMPLITUDE, -0.95, 0.95)
		var pan: float = sin(TAU * bar_phase) * 0.18
		var left := int(clamp(mono * (1.0 - pan), -0.95, 0.95) * 32767.0)
		var right := int(clamp(mono * (1.0 + pan), -0.95, 0.95) * 32767.0)
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
