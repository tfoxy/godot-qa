use std::{
    collections::HashMap,
    env, fs,
    io::{self, Write},
    path::Path,
    process::Command,
    thread,
    time::Duration,
};

fn main() {
    let config = fs::read_to_string(env::current_exe().unwrap().with_extension("fixture")).unwrap();
    let values: HashMap<String, String> = config
        .lines()
        .map(|line| {
            let (key, hex) = line.split_once('=').unwrap();
            let bytes: Vec<u8> = hex
                .as_bytes()
                .chunks_exact(2)
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect();
            (key.to_owned(), String::from_utf8(bytes).unwrap())
        })
        .collect();
    let get = |key: &str| values.get(key).map(String::as_str).unwrap_or("");
    if !get("remove_file").is_empty() {
        match fs::remove_file(get("remove_file")) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => panic!("remove fixture file: {error}"),
        }
    }
    let args: Vec<String> = env::args().skip(1).collect();
    if let Some(response) = values.get(&format!("response:{}", args.join(" "))) {
        print!("{response}");
        return;
    }
    if args.iter().any(|arg| arg == "--version") && !get("version").is_empty() {
        println!("{}", get("version"));
        return;
    }
    if !get("request_prefix").is_empty() {
        let Some(request) = args
            .iter()
            .find_map(|arg| arg.strip_prefix(get("request_prefix")))
        else {
            return;
        };
        let parent = Path::new(request).parent().unwrap();
        let request_json = if get("scenario_proof") == "passed" {
            fs::read_to_string(request).unwrap()
        } else {
            String::new()
        };
        for (key, value) in &values {
            if let Some(relative) = key.strip_prefix("write:") {
                let output = parent.join(relative);
                fs::create_dir_all(output.parent().unwrap()).unwrap();
                let value = if get("scenario_proof") == "passed" && relative == "result.json" {
                    with_scenario_proof(value, &request_json)
                } else {
                    value.clone()
                };
                fs::write(output, value).unwrap();
            }
        }
        // Apply path-specific results after defaults. Match native path components so
        // scenario names behave identically with Windows and Unix separators.
        for (key, value) in &values {
            if let Some(rule) = key.strip_prefix("write_match:") {
                let (component, relative) = rule
                    .split_once(':')
                    .expect("write_match:component:relative-output");
                if parent
                    .components()
                    .any(|part| part.as_os_str() == component)
                {
                    let output = parent.join(relative);
                    fs::create_dir_all(output.parent().unwrap()).unwrap();
                    let value = if get("scenario_proof") == "passed" && relative == "result.json" {
                        with_scenario_proof(value, &request_json)
                    } else {
                        value.clone()
                    };
                    fs::write(output, value).unwrap();
                }
            }
        }
    }
    if !get("argv_file").is_empty() {
        fs::write(get("argv_file"), args.join("\n") + "\n").unwrap();
    }
    let arg_value = |flag: &str| {
        args.windows(2)
            .find(|pair| pair[0] == flag)
            .map(|pair| pair[1].as_str())
    };
    if get("mode") == "diagnostics" {
        if let Some(project) = arg_value("--path") {
            fs::write(
                Path::new(project).join("project.godot"),
                "[application]\nconfig/name=\"Mutated By Fake Godot\"\n",
            )
            .unwrap();
        }
        if let Some(log) = arg_value("--log-file") {
            fs::create_dir_all(Path::new(log).parent().unwrap()).unwrap();
            fs::write(log, get("log")).unwrap();
        }
    }
    if get("mode") == "wrapper" {
        let args: Vec<_> = args
            .iter()
            .filter(|arg| {
                !arg.starts_with("--wrapper-")
                    && !arg.starts_with("--global-")
                    && !arg.starts_with("--display-")
            })
            .collect();
        let status = Command::new(args[0]).args(&args[1..]).status();
        std::process::exit(status.ok().and_then(|status| status.code()).unwrap_or(127));
    }
    if !get("child_exe").is_empty() {
        let mut child = Command::new(get("child_exe")).spawn().unwrap();
        if !get("child_pid_file").is_empty() {
            fs::write(get("child_pid_file"), child.id().to_string()).unwrap();
        }
        if get("wait_child") == "true" {
            child.wait().unwrap();
        }
    }
    print!(
        "{}",
        get("stdout").replace("{arg1}", args.first().map(String::as_str).unwrap_or(""))
    );
    eprint!("{}", get("stderr"));
    io::stdout().flush().unwrap();
    io::stderr().flush().unwrap();
    if let Ok(milliseconds) = get("sleep_ms").parse() {
        thread::sleep(Duration::from_millis(milliseconds));
    }
    std::process::exit(get("exit").parse().unwrap_or(0));
}

// This helper is compiled directly with rustc so subprocess fixtures stay portable
// and dependency-free. Slice JSON values without interpreting arbitrary strings.
fn json_value_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut index = start;
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    while let Some(&byte) = bytes.get(index) {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
                if depth == 0 {
                    return Some(index + 1);
                }
            }
        } else {
            match byte {
                b'"' => quoted = true,
                b'{' | b'[' => depth += 1,
                b'}' | b']' if depth > 0 => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(index + 1);
                    }
                }
                b',' | b'}' | b']' if depth == 0 => return Some(index),
                _ => {}
            }
        }
        index += 1;
    }
    (!quoted && depth == 0).then_some(index)
}

fn json_fields(text: &str) -> Option<Vec<(&str, &str)>> {
    let text = text.trim();
    if !text.starts_with('{') || !text.ends_with('}') {
        return None;
    }
    let bytes = text.as_bytes();
    let mut index = 1usize;
    let mut fields = Vec::new();
    loop {
        while bytes
            .get(index)
            .is_some_and(|b| b.is_ascii_whitespace() || *b == b',')
        {
            index += 1;
        }
        if bytes.get(index) == Some(&b'}') {
            return Some(fields);
        }
        if bytes.get(index) != Some(&b'"') {
            return None;
        }
        let key_end = json_value_end(bytes, index)?;
        let key = text.get(index + 1..key_end - 1)?;
        index = key_end;
        while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
            index += 1;
        }
        if bytes.get(index) != Some(&b':') {
            return None;
        }
        index += 1;
        while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
            index += 1;
        }
        let end = json_value_end(bytes, index)?;
        fields.push((key, text.get(index..end)?.trim()));
        index = end;
    }
}

fn json_field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    json_fields(text)?
        .into_iter()
        .find(|(name, _)| *name == key)
        .map(|(_, value)| value)
}

fn with_scenario_proof(result: &str, request: &str) -> String {
    let Some(fields) = json_fields(result) else {
        return result.to_owned();
    };
    let Some(run) = json_field(request, "run") else {
        return result.to_owned();
    };
    let Some(id) = json_field(run, "id") else {
        return result.to_owned();
    };
    let Some(nonce) = json_field(run, "nonce") else {
        return result.to_owned();
    };
    let Some(manifest) = json_field(request, "assertion_proof") else {
        return result.to_owned();
    };
    let Some(entries) = json_field(manifest, "entries") else {
        return result.to_owned();
    };
    let Some(excluded) = json_field(manifest, "cli_assertions_excluded") else {
        return result.to_owned();
    };
    let mut index = 1usize;
    let mut completed = Vec::new();
    while index < entries.len() {
        while entries
            .as_bytes()
            .get(index)
            .is_some_and(|b| b.is_ascii_whitespace() || *b == b',')
        {
            index += 1;
        }
        if entries.as_bytes().get(index) == Some(&b']') {
            break;
        }
        let Some(end) = json_value_end(entries.as_bytes(), index) else {
            return result.to_owned();
        };
        let entry = entries[index..end].trim();
        if json_fields(entry).is_none() {
            return result.to_owned();
        }
        completed.push(format!(
            "{},\"status\":\"passed\"}}",
            &entry[..entry.len() - 1]
        ));
        index = end;
    }
    let count = completed.len();
    let mut additions = Vec::new();
    if !fields.iter().any(|(key, _)| *key == "run_identity") {
        additions.push(format!(
            "\"run_identity\":{{\"run_id\":{id},\"nonce\":{nonce}}}"
        ));
    }
    if !fields.iter().any(|(key, _)| *key == "assertion_coverage") {
        additions.push(format!("\"assertion_coverage\":{{\"schema_version\":1,\"run_id\":{id},\"nonce\":{nonce},\"expected\":{count},\"started\":{count},\"finished\":{count},\"passed\":{count},\"failed\":0,\"complete\":true,\"cli_assertions_excluded\":{excluded},\"entries\":[{}]}}", completed.join(",")));
    }
    if additions.is_empty() {
        return result.to_owned();
    }
    let result = result.trim();
    format!(
        "{}{}{} }}",
        &result[..result.len() - 1],
        if fields.is_empty() { "" } else { "," },
        additions.join(",")
    )
}
