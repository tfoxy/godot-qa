use super::*;
use godot_qa_core::audio::{DecodedWav, WavMetadata};

const MAX_AUDIO_FILES: usize = 64;
const MAX_AUDIO_BYTES: u64 = 512 * 1024 * 1024;
const MAX_WAV_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Default)]
pub(super) struct PacketBudget {
    files: usize,
    bytes: u64,
}

impl PacketBudget {
    fn reserve(&mut self, path: &Path, bytes: u64) -> Result<(), ExportError> {
        let files = self.files.checked_add(1);
        let total = self.bytes.checked_add(bytes);
        match (files, total) {
            (Some(files), Some(total)) if files <= MAX_AUDIO_FILES && total <= MAX_AUDIO_BYTES => {
                self.files = files;
                self.bytes = total;
                Ok(())
            }
            _ => Err((
                path.to_path_buf(),
                "export_audio_resource_limit",
                "selected audio exceeds packet limit: 64 WAV files and 512 MiB".to_owned(),
            )),
        }
    }
}

pub(super) struct PreparedAudio {
    id: String,
    source: PathBuf,
    source_relative: String,
    hash: String,
    metadata: WavMetadata,
    review: Value,
}

pub(super) fn prepare_audio(
    value: &Value,
    root: &Path,
    budget: &mut PacketBudget,
) -> Result<Vec<PreparedAudio>, ExportError> {
    let Some(items) = value.get("audio") else {
        return Ok(Vec::new());
    };
    let items = items.as_array().ok_or_else(|| {
        (
            root.to_path_buf(),
            "export_selection_invalid",
            "audio must be an array".to_owned(),
        )
    })?;
    if items.len() > MAX_AUDIO_FILES {
        return Err((
            root.to_path_buf(),
            "export_audio_resource_limit",
            "a packet permits at most 64 WAV files".to_owned(),
        ));
    }
    let mut seen = HashSet::new();
    let mut audio = Vec::new();
    for item in items {
        if budget.files >= MAX_AUDIO_FILES {
            return Err((
                root.to_path_buf(),
                "export_audio_resource_limit",
                "a packet permits at most 64 WAV files".to_owned(),
            ));
        }
        let id = required_slug(item, "id", root)?;
        if !seen.insert(id.clone()) {
            return Err((
                root.to_path_buf(),
                "export_selection_invalid",
                format!("duplicate audio id {id}"),
            ));
        }
        if item.get("role").and_then(Value::as_str) != Some("original") {
            return Err((
                root.to_path_buf(),
                "export_selection_invalid",
                "audio role must be original".to_owned(),
            ));
        }
        let relative = item.get("path").and_then(Value::as_str).ok_or_else(|| {
            (
                root.to_path_buf(),
                "export_selection_invalid",
                "audio needs path".to_owned(),
            )
        })?;
        let path = confined_file(root, relative)?;
        if path.extension().and_then(|ext| ext.to_str()) != Some("wav") {
            return Err((
                path,
                "export_audio_invalid",
                "selected audio must be WAV".to_owned(),
            ));
        }
        // Derive metadata and identity from the same bounded byte buffer.
        let file = File::open(&path)
            .map_err(|error| (path.clone(), "export_audio_invalid", error.to_string()))?;
        let mut bytes = Vec::new();
        file.take(MAX_WAV_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| (path.clone(), "export_audio_invalid", error.to_string()))?;
        if bytes.len() as u64 > MAX_WAV_BYTES {
            return Err((
                path,
                "export_audio_resource_limit",
                "selected WAV exceeds 64 MiB".to_owned(),
            ));
        }
        budget.reserve(&path, bytes.len() as u64)?;
        let metadata = DecodedWav::from_bytes(&bytes)
            .map_err(|error| (path.clone(), "export_audio_invalid", error.message))?
            .metadata();
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let hash = to_lower_hex(&hasher.finish());
        let review = selected_review(item.get("review"), &hash, &path)?;
        audio.push(PreparedAudio {
            id,
            source: path,
            source_relative: relative.to_owned(),
            hash,
            metadata,
            review,
        });
    }
    Ok(audio)
}

pub(super) fn validate_packet(sources: &[PreparedSource], path: &Path) -> Result<(), ExportError> {
    let count: usize = sources.iter().map(|source| source.audio.len()).sum();
    let bytes: u64 = sources
        .iter()
        .flat_map(|source| &source.audio)
        .map(|audio| audio.metadata.byte_count as u64)
        .sum();
    if count > MAX_AUDIO_FILES || bytes > MAX_AUDIO_BYTES {
        return Err((
            path.to_path_buf(),
            "export_audio_resource_limit",
            format!(
                "selected audio exceeds packet limit: {count}/64 WAV files, {bytes}/{MAX_AUDIO_BYTES} bytes"
            ),
        ));
    }
    Ok(())
}

fn bounded_hash(path: &Path) -> Result<String, ExportError> {
    let file = File::open(path).map_err(|error| {
        (
            path.to_path_buf(),
            "export_audio_invalid",
            error.to_string(),
        )
    })?;
    let mut reader = file.take(MAX_WAV_BYTES + 1);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut size = 0_u64;
    loop {
        let count = reader.read(&mut buffer).map_err(|error| {
            (
                path.to_path_buf(),
                "export_audio_invalid",
                error.to_string(),
            )
        })?;
        if count == 0 {
            break;
        }
        size += count as u64;
        if size > MAX_WAV_BYTES {
            return Err((
                path.to_path_buf(),
                "export_audio_resource_limit",
                "selected WAV exceeds 64 MiB".to_owned(),
            ));
        }
        hasher.update(&buffer[..count]);
    }
    Ok(to_lower_hex(&hasher.finish()))
}

pub(super) fn write_audio(
    output: &Path,
    source_id: &str,
    items: Vec<PreparedAudio>,
) -> Result<Vec<Value>, ExportError> {
    let mut entries = Vec::new();
    for item in items {
        let relative = format!("sources/{source_id}/audio/{}.wav", item.id);
        let destination = output.join(&relative);
        // Verify the entire bounded source before creating the destination so
        // changed sources cannot leave unexpected media in a partial packet.
        let source = File::open(&item.source).map_err(|error| {
            (
                item.source.clone(),
                "export_write_failed",
                error.to_string(),
            )
        })?;
        let mut bytes = Vec::new();
        source
            .take(MAX_WAV_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| {
                (
                    item.source.clone(),
                    "export_write_failed",
                    error.to_string(),
                )
            })?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        if bytes.len() as u64 > MAX_WAV_BYTES
            || bytes.len() != item.metadata.byte_count
            || to_lower_hex(&hasher.finish()) != item.hash
        {
            return Err((
                item.source,
                "export_hash_mismatch",
                "source WAV changed after selection validation".to_owned(),
            ));
        }
        fs::create_dir_all(destination.parent().unwrap()).map_err(|error| {
            (
                destination.clone(),
                "export_write_failed",
                error.to_string(),
            )
        })?;
        write_new(&destination, &bytes).map_err(|error| {
            (
                destination.clone(),
                "export_write_failed",
                error.to_string(),
            )
        })?;
        let export_hash = bounded_hash(&destination)?;
        if export_hash != item.hash {
            return Err((
                destination,
                "export_hash_mismatch",
                "exported WAV hash differs from source".to_owned(),
            ));
        }
        entries.push(json!({ "id": item.id, "path": relative, "source_relative_path": item.source_relative,
            "role": "original", "metadata": item.metadata, "source_sha256": item.hash,
            "export_sha256": export_hash, "transformation": "byte_exact_copy", "audio_review": item.review }));
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_budget_rejects_aggregate_bytes_before_decoding_or_writing() {
        let mut budget = PacketBudget::default();
        for _ in 0..8 {
            budget
                .reserve(Path::new("selected.wav"), MAX_WAV_BYTES)
                .unwrap();
        }
        assert!(budget.reserve(Path::new("too-much.wav"), 1).is_err());
        assert_eq!(budget.bytes, MAX_AUDIO_BYTES);
        assert_eq!(budget.files, 8);
    }

    #[test]
    fn changed_source_is_rejected_before_creating_exported_wav() {
        let root = std::env::temp_dir().join(format!(
            "qa-wav-replacement-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("candidate.wav");
        let original = b"selected WAV bytes";
        fs::write(&source, original).unwrap();
        let mut hash = Sha256::new();
        hash.update(original);
        let item = PreparedAudio {
            id: "candidate".to_owned(),
            source: source.clone(),
            source_relative: "candidate.wav".to_owned(),
            hash: to_lower_hex(&hash.finish()),
            metadata: WavMetadata {
                byte_count: original.len(),
                duration_seconds: 1.0,
                sample_rate: 8000,
                channels: 1,
                bits_per_sample: 16,
                encoding: "pcm",
            },
            review: json!({"status":"unreviewed"}),
        };
        fs::write(&source, b"unexpected replacement bytes").unwrap();
        let output = root.join("packet");
        let (_, kind, _) = write_audio(&output, "run", vec![item]).unwrap_err();
        assert_eq!(kind, "export_hash_mismatch");
        assert!(!output.join("sources/run/audio/candidate.wav").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
