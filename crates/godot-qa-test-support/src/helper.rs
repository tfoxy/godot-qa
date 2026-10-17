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
        for (key, value) in &values {
            if let Some(relative) = key.strip_prefix("write:") {
                let output = parent.join(relative);
                fs::create_dir_all(output.parent().unwrap()).unwrap();
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
