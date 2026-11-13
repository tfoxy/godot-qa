// Native fake Godot: return valid check results, then block the aggregate result path.
use std::{env, fs, path::Path};
fn main() {
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|arg| arg == "--version") {
        println!("4.5.2.stable");
        return;
    }
    if let Some(request) = args.iter().find_map(|arg| arg.strip_prefix("--qa-check-request=")) {
        let parent = Path::new(request).parent().unwrap();
        let result = if parent.parent().unwrap().file_name().unwrap() == "scripts" {
            r#"{"ok":true,"kind":"script_compile","target":"res://probe.gd","runtime_errors":[],"failures":[]}"#
        } else {
            r#"{"ok":true,"scene":"res://probe.tscn","runtime_errors":[],"failures":[]}"#
        };
        fs::write(parent.join("result.json"), result).unwrap();
        fs::create_dir_all(parent.parent().unwrap().parent().unwrap().join("result.json")).unwrap();
    } else {
        let log = args.windows(2).find(|pair| pair[0] == "--log-file").unwrap();
        let parent = Path::new(&log[1]).parent().unwrap();
        // Check's import is followed by a scene run; block only standalone import here.
        if parent.file_name().unwrap() != "import" {
            fs::create_dir_all(parent.join("result.json")).unwrap();
        }
    }
}
