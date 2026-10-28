/// Record the invocation's installed-content comparison without rehashing for
/// each child. This intentionally makes no claim about a running session's code.
fn addon_provenance(config: &ProjectConfig, artifacts: &artifacts::RunArtifacts) -> Value {
    let mut identity = json!(config.installed_addon_identity());
    let update = || -> io::Result<()> {
        let mut meta: Value = serde_json::from_slice(&fs::read(&artifacts.meta_path)?)?;
        let object = meta
            .as_object_mut()
            .ok_or_else(|| io::Error::other("run metadata is not an object"))?;
        object.insert("addon_identity".to_string(), identity.clone());
        write_json(&artifacts.meta_path, &meta)
    };
    if let Err(error) = update() {
        identity["metadata_write_error"] = json!({
            "path": artifacts.meta_path,
            "message": error.to_string(),
        });
    }
    identity
}
