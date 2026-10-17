use super::{Result, checks};
use std::fs;
use std::path::Path;

const CLIFF_VERSION: &str = "2.10.1";
const RELEASE_FILES: [&str; 4] = [
    "Cargo.toml",
    "Cargo.lock",
    "addons/godot_qa/plugin.cfg",
    "CHANGELOG.md",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Version(u64, u64, u64);

impl Version {
    fn parse(value: &str) -> Result<Self> {
        let parts = value.split('.').collect::<Vec<_>>();
        if parts.len() != 3
            || parts.iter().any(|part| {
                part.is_empty()
                    || !part.bytes().all(|byte| byte.is_ascii_digit())
                    || part.len() > 1 && part.starts_with('0')
            })
        {
            return Err(format!("expected a stable semantic version, got {value}").into());
        }
        Ok(Self(
            parts[0].parse()?,
            parts[1].parse()?,
            parts[2].parse()?,
        ))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Bump {
    None,
    Patch,
    Minor,
    Breaking,
}

fn classify(message: &str) -> Result<Bump> {
    let title = message.lines().next().unwrap_or("");
    let (header, subject) = title
        .split_once(": ")
        .ok_or("commit subject must use Conventional Commits: type(scope): description")?;
    if subject.trim().is_empty() {
        return Err("commit subject has no description".into());
    }
    let breaking = header.ends_with('!')
        || message.lines().any(|line| {
            line.starts_with("BREAKING CHANGE: ") || line.starts_with("BREAKING-CHANGE: ")
        });
    let header = header.strip_suffix('!').unwrap_or(header);
    let kind = match header.split_once('(') {
        Some((kind, scope))
            if scope.ends_with(')')
                && scope.len() > 1
                && !scope[..scope.len() - 1]
                    .chars()
                    .any(|ch| ch.is_whitespace() || matches!(ch, '(' | ')' | '!')) =>
        {
            kind
        }
        Some(_) => return Err("invalid Conventional Commit scope".into()),
        None => header,
    };
    if !matches!(
        kind,
        "feat"
            | "fix"
            | "perf"
            | "docs"
            | "test"
            | "refactor"
            | "build"
            | "ci"
            | "chore"
            | "revert"
            | "style"
    ) {
        return Err(format!("unsupported Conventional Commit type: {kind}").into());
    }
    Ok(if breaking {
        Bump::Breaking
    } else {
        match kind {
            "feat" => Bump::Minor,
            "fix" | "perf" => Bump::Patch,
            _ => Bump::None,
        }
    })
}

pub fn validate_commit_file(path: &Path) -> Result {
    classify(&fs::read_to_string(path)?)?;
    Ok(())
}

pub fn current_version() -> Result<String> {
    current_version_at(Path::new("."))
}

fn current_version_at(directory: &Path) -> Result<String> {
    let text = fs::read_to_string(directory.join("Cargo.toml"))?;
    let section = text
        .split_once("[workspace.package]")
        .ok_or("missing workspace package")?
        .1;
    let value = section
        .lines()
        .take_while(|line| !line.starts_with('['))
        .find_map(|line| line.trim().strip_prefix("version = "))
        .ok_or("missing workspace version")?
        .trim_matches('"')
        .to_string();
    Version::parse(&value)?;
    Ok(value)
}

pub fn verify_version(tag: &str) -> Result {
    verify_version_at(Path::new("."), tag)
}

fn verify_version_at(directory: &Path, tag: &str) -> Result {
    let version = current_version_at(directory)?;
    if tag != format!("v{version}") {
        return Err(format!("tag {tag} does not match workspace v{version}").into());
    }
    let addon = fs::read_to_string(directory.join("addons/godot_qa/plugin.cfg"))?;
    if !addon
        .lines()
        .any(|line| line == format!("version=\"{version}\""))
    {
        return Err("addon version differs from workspace version".into());
    }
    Ok(())
}

pub fn verify_release(tag: &str) -> Result {
    verify_release_at(Path::new("."), tag)
}

fn verify_release_at(directory: &Path, tag: &str) -> Result {
    verify_version_at(directory, tag)?;
    let heading = format!("## [{}]", &tag[1..]);
    if !fs::read_to_string(directory.join("CHANGELOG.md"))?
        .lines()
        .any(|line| line == heading)
    {
        return Err("changelog has no matching release heading".into());
    }
    Ok(())
}

pub fn check_commits(base: &str, head: &str) -> Result {
    check_commits_at(Path::new("."), base, head)
}

fn check_commits_at(directory: &Path, base: &str, head: &str) -> Result {
    // GitHub sends all-zero `before` for an initial branch push. Check its tip,
    // never import private/pre-publication ancestry into the public convention.
    // A force-pushed replacement root also has no ancestry to compare. The
    // previous root may not exist in a fresh CI checkout, so validate this tip.
    let head_is_root = git_at(directory, &["rev-list", "--parents", "-n", "1", head])?
        .split_whitespace()
        .count()
        == 1;
    let range = if head_is_root || base.is_empty() || base.bytes().all(|ch| ch == b'0') {
        head.to_string()
    } else {
        format!("{base}..{head}")
    };
    let messages = if range == head {
        git_at(directory, &["log", "-1", "--format=%B%x00", head])?
    } else {
        git_at(
            directory,
            &["log", "--no-merges", "--format=%B%x00", &range],
        )?
    };
    for message in messages
        .split('\0')
        .map(str::trim)
        .filter(|message| !message.is_empty())
    {
        classify(message)?;
    }
    Ok(())
}

#[derive(Debug)]
struct Plan {
    version: Version,
    previous: Option<String>,
    bootstrap: bool,
}

fn plan(explicit: Option<&str>) -> Result<Plan> {
    plan_at(Path::new("."), explicit)
}

fn git_at(directory: &Path, args: &[&str]) -> Result<String> {
    output_at(directory, "git", args)
}

fn output_at(directory: &Path, program: &str, args: &[&str]) -> Result<String> {
    let result = std::process::Command::new(program)
        .args(args)
        .current_dir(directory)
        .output()?;
    if !result.status.success() {
        return Err(format!(
            "{program} {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&result.stderr)
        )
        .into());
    }
    Ok(String::from_utf8(result.stdout)?.trim().to_string())
}

fn plan_at(directory: &Path, explicit: Option<&str>) -> Result<Plan> {
    let current = Version::parse(&current_version_at(directory)?)?;
    let tags = git_at(
        directory,
        &[
            "tag",
            "--merged",
            "HEAD",
            "--list",
            "v*",
            "--sort=-version:refname",
        ],
    )?;
    let previous = tags
        .lines()
        .find(|tag| {
            tag.strip_prefix('v')
                .is_some_and(|v| Version::parse(v).is_ok())
        })
        .map(str::to_owned);
    let Some(tag) = previous else {
        let version = explicit
            .map(Version::parse)
            .transpose()?
            .unwrap_or(Version(0, 1, 0));
        if version != Version(0, 1, 0) || current != version {
            return Err("bootstrap release must be workspace version 0.1.0".into());
        }
        return Ok(Plan {
            version,
            previous: None,
            bootstrap: true,
        });
    };
    let previous_version = Version::parse(&tag[1..])?;
    if current != previous_version {
        return Err(format!("workspace {current} differs from latest release {tag}; finish or review the prepared release first").into());
    }
    let messages = git_at(
        directory,
        &[
            "log",
            "--no-merges",
            "--format=%B%x00",
            &format!("{tag}..HEAD"),
        ],
    )?;
    let mut bump = Bump::None;
    for message in messages
        .split('\0')
        .map(str::trim)
        .filter(|m| !m.is_empty())
    {
        bump = bump.max(classify(message)?);
    }
    let version = if let Some(explicit) = explicit {
        let version = Version::parse(explicit)?;
        if version != Version(1, 0, 0) || previous_version.0 != 0 {
            return Err("explicit --version is reserved for deliberate 1.0.0 graduation".into());
        }
        version
    } else {
        match bump {
            Bump::None => return Err("no releasable changes since last tag (feat, fix, perf, or breaking change required)".into()),
            Bump::Patch => Version(previous_version.0, previous_version.1, previous_version.2 + 1),
            Bump::Minor => Version(previous_version.0, previous_version.1 + 1, 0),
            Bump::Breaking if previous_version.0 == 0 => Version(0, previous_version.1 + 1, 0),
            Bump::Breaking => Version(previous_version.0 + 1, 0, 0),
        }
    };
    Ok(Plan {
        version,
        previous: Some(tag),
        bootstrap: false,
    })
}

pub fn run(args: &[String]) -> Result {
    let action = args.first().map(String::as_str).unwrap_or("--dry-run");
    let explicit = match args.get(1..) {
        Some([]) | None => None,
        Some([flag, version]) if flag == "--version" => Some(version.as_str()),
        _ => {
            return Err(
                "usage: cargo xtask release [--dry-run | prepare | tag] [--version 1.0.0]".into(),
            );
        }
    };
    match action {
        "--dry-run" | "prepare" => {
            let plan = plan(explicit)?;
            println!(
                "{}: {} -> v{}\nRelease commit: chore(release): v{}\nNo push is performed.",
                if action == "--dry-run" {
                    "Dry run"
                } else {
                    "Prepare"
                },
                plan.previous
                    .as_deref()
                    .unwrap_or("public bootstrap (old history excluded)"),
                plan.version,
                plan.version
            );
            if action == "--dry-run" {
                return Ok(());
            }
            prepare_at(Path::new("."), &plan, checks, changelog_at)?;
            println!(
                "Prepared v{}. Review the commit, then run cargo xtask release tag explicitly.",
                plan.version
            );
            Ok(())
        }
        "tag" if explicit.is_none() => tag_at(Path::new(".")),
        _ => Err("unknown release action; use --dry-run, prepare, or tag".into()),
    }
}

/// Keep checks injectable so regression tests execute the real file/Git flow
/// without launching the entire workspace and Godot suite in disposable repos.
fn prepare_at(
    directory: &Path,
    plan: &Plan,
    mut check: impl FnMut() -> Result,
    render_changelog: impl FnOnce(&Path, &Plan) -> Result<String>,
) -> Result {
    ensure_clean_at(directory)?;
    // Render in memory before touching versions. Bootstrap rendering deliberately
    // ignores all private pre-publication commit history.
    let changelog = render_changelog(directory, plan)?;
    check()?;
    let current = current_version_at(directory)?;
    let next = plan.version.to_string();
    let manifest_path = directory.join("Cargo.toml");
    let manifest = fs::read_to_string(&manifest_path)?;
    fs::write(
        &manifest_path,
        manifest.replacen(
            &format!("version = \"{current}\""),
            &format!("version = \"{next}\""),
            1,
        ),
    )?;
    let addon_path = directory.join("addons/godot_qa/plugin.cfg");
    let addon = fs::read_to_string(&addon_path)?;
    fs::write(
        &addon_path,
        addon.replacen(
            &format!("version=\"{current}\""),
            &format!("version=\"{next}\""),
            1,
        ),
    )?;
    update_lock_at(directory, &current, &next)?;
    fs::write(directory.join("CHANGELOG.md"), changelog)?;
    // A failed second gate preserves reviewable changes and never reaches staging,
    // commit creation, or tagging. The tag always requires a separate command.
    verify_release_at(directory, &format!("v{next}"))?;
    check()?;
    let mut add = vec!["add", "--"];
    add.extend(RELEASE_FILES);
    git_at(directory, &add)?;
    git_at(
        directory,
        &["commit", "-m", &format!("chore(release): v{next}")],
    )?;
    Ok(())
}

fn tag_at(directory: &Path) -> Result {
    ensure_clean_at(directory)?;
    let version = current_version_at(directory)?;
    verify_release_at(directory, &format!("v{version}"))?;
    let subject = git_at(directory, &["log", "-1", "--format=%s"])?;
    if subject != format!("chore(release): v{version}") {
        return Err("HEAD must be the prepared release commit".into());
    }
    git_at(
        directory,
        &[
            "tag",
            "-a",
            &format!("v{version}"),
            "-m",
            &format!("godot-qa {version}"),
        ],
    )?;
    Ok(())
}

fn ensure_clean_at(directory: &Path) -> Result {
    if !git_at(directory, &["status", "--porcelain"])?.is_empty() {
        return Err("release requires a clean working tree and index; commit or preserve your changes first".into());
    }
    Ok(())
}

fn changelog_at(directory: &Path, plan: &Plan) -> Result<String> {
    if plan.bootstrap {
        return Ok("# Changelog\n\n## [0.1.0]\n\nInitial public release of godot-qa: a Rust CLI and Godot addon for reproducible scenarios, live inspection, screenshots, layout audits, and structured assertions.\n\nIncludes optimized Windows and Linux x64 release bundles.\n".into());
    }
    let cliff = output_at(directory, "git-cliff", &["--version"]).map_err(|_| {
        format!(
            "install pinned git-cliff: cargo install git-cliff --version {CLIFF_VERSION} --locked"
        )
    })?;
    if cliff.split_whitespace().nth(1) != Some(CLIFF_VERSION) {
        return Err(format!("expected git-cliff {CLIFF_VERSION}, got {cliff}").into());
    }
    let section = output_at(
        directory,
        "git-cliff",
        &[
            "--config",
            "cliff.toml",
            "--tag",
            &format!("v{}", plan.version),
            "--strip",
            "header",
            &format!("{}..HEAD", plan.previous.as_deref().unwrap()),
        ],
    )?;
    let existing = fs::read_to_string(directory.join("CHANGELOG.md"))?;
    Ok(format!(
        "# Changelog\n\n{}\n\n{}",
        section.trim(),
        existing
            .strip_prefix("# Changelog")
            .unwrap_or(&existing)
            .trim_start()
    ))
}

fn update_lock_at(directory: &Path, current: &str, next: &str) -> Result {
    let lock = fs::read_to_string(directory.join("Cargo.lock"))?;
    let mut updated = String::new();
    for (index, block) in lock.split("[[package]]").enumerate() {
        if index > 0 {
            updated.push_str("[[package]]");
        }
        if block
            .lines()
            .any(|line| line.starts_with("name = \"godot-qa-"))
            && !block.lines().any(|line| line.starts_with("source = "))
        {
            updated.push_str(&block.replacen(
                &format!("version = \"{current}\""),
                &format!("version = \"{next}\""),
                1,
            ));
        } else {
            updated.push_str(block);
        }
    }
    fs::write(directory.join("Cargo.lock"), updated)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conventional_messages_classify_releases() {
        assert_eq!(classify("feat(cli): inspect scene").unwrap(), Bump::Minor);
        assert_eq!(classify("fix: retain exit code").unwrap(), Bump::Patch);
        assert_eq!(classify("perf: reduce allocations").unwrap(), Bump::Patch);
        assert_eq!(classify("docs: explain setup").unwrap(), Bump::None);
        assert_eq!(classify("feat!: change protocol").unwrap(), Bump::Breaking);
        assert_eq!(
            classify("refactor: protocol\n\nBREAKING CHANGE: removed field").unwrap(),
            Bump::Breaking
        );
        assert!(classify("Improve random stuff").is_err());
        assert!(classify("fix(): bad scope").is_err());
        assert!(classify("fix( ): bad scope").is_err());
        assert!(classify("feat!!: malformed").is_err());
        assert_eq!(classify("revert: undo change").unwrap(), Bump::None);
        assert!(classify("fix: ").is_err());
    }
    #[test]
    fn stable_versions_are_strict() {
        assert_eq!(Version::parse("0.1.0").unwrap(), Version(0, 1, 0));
        for invalid in ["v0.1.0", "1.2", "1.2.3.4", "1.2.3-beta", "01.0.0"] {
            assert!(Version::parse(invalid).is_err());
        }
    }

    #[test]
    fn commit_check_validates_replacement_root_without_previous_commit() {
        let previous = Repo::new("0.1.0");
        previous.git(&["commit", "--amend", "-m", "chore: previous root"]);
        let previous_head = previous.git(&["rev-parse", "HEAD"]);
        let replacement = Repo::new("0.1.0");
        assert!(git_at(&replacement.0, &["cat-file", "-e", &previous_head]).is_err());
        let head = replacement.git(&["rev-parse", "HEAD"]);
        assert!(check_commits_at(&replacement.0, &previous_head, &head).is_err());
        replacement.git(&["commit", "--amend", "-m", "chore(release): v0.1.0"]);
        let head = replacement.git(&["rev-parse", "HEAD"]);
        check_commits_at(&replacement.0, &previous_head, &head).unwrap();
    }

    struct Repo(std::path::PathBuf);
    impl Repo {
        fn new(version: &str) -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("qa release git {} {unique}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            let repo = Self(path);
            repo.git(&["init", "--initial-branch=main"]);
            repo.git(&["config", "user.name", "Release test"]);
            repo.git(&["config", "user.email", "release-test@example.invalid"]);
            repo.git(&["config", "commit.gpgsign", "false"]);
            repo.git(&["config", "tag.gpgsign", "false"]);
            repo.git(&["config", "core.hooksPath", "disabled-test-hooks"]);
            fs::write(
                repo.0.join("Cargo.toml"),
                format!("[workspace.package]\nversion = \"{version}\"\n"),
            )
            .unwrap();
            fs::create_dir_all(repo.0.join("addons/godot_qa")).unwrap();
            fs::write(
                repo.0.join("addons/godot_qa/plugin.cfg"),
                format!("[plugin]\nversion=\"{version}\"\n"),
            )
            .unwrap();
            fs::write(repo.0.join("Cargo.lock"), format!("version = 4\n\n[[package]]\nname = \"godot-qa-core\"\nversion = \"{version}\"\n")).unwrap();
            fs::write(
                repo.0.join("CHANGELOG.md"),
                "# Changelog\n\nPRIVATE_CHANGELOG_SENTINEL\n",
            )
            .unwrap();
            repo.commit("private legacy history is excluded");
            repo
        }
        fn git(&self, args: &[&str]) -> String {
            git_at(&self.0, args).unwrap()
        }
        fn commit(&self, message: &str) {
            self.git(&["add", "."]);
            self.git(&["commit", "--allow-empty", "-m", message]);
        }
    }
    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn git_bootstrap_excludes_private_history_and_leaves_repo_unchanged() {
        let repo = Repo::new("0.1.0");
        let before = repo.git(&["rev-parse", "HEAD"]);
        let plan = plan_at(&repo.0, None).unwrap();
        assert_eq!(plan.version, Version(0, 1, 0));
        assert!(plan.bootstrap);
        assert_eq!(repo.git(&["rev-parse", "HEAD"]), before);
        assert!(repo.git(&["status", "--porcelain"]).is_empty());
        assert!(repo.git(&["tag"]).is_empty());
        fs::write(repo.0.join("dirty.txt"), "uncommitted").unwrap();
        assert!(ensure_clean_at(&repo.0).is_err());
    }

    #[test]
    fn git_release_policy_handles_patch_feature_breaking_and_graduation() {
        for (current, message, explicit, expected) in [
            ("0.1.0", "fix: path quoting", None, "0.1.1"),
            ("0.1.0", "perf: faster tree", None, "0.1.1"),
            ("0.1.0", "feat: new command", None, "0.2.0"),
            ("0.1.0", "feat!: new protocol", None, "0.2.0"),
            ("1.2.3", "feat!: new protocol", None, "2.0.0"),
            ("0.7.0", "docs: stable contract", Some("1.0.0"), "1.0.0"),
        ] {
            let repo = Repo::new(current);
            repo.git(&["tag", &format!("v{current}")]);
            repo.commit(message);
            assert_eq!(
                plan_at(&repo.0, explicit).unwrap().version.to_string(),
                expected
            );
            assert!(repo.git(&["status", "--porcelain"]).is_empty());
        }
    }

    #[test]
    fn git_release_rejects_nonreleasable_or_malformed_commits() {
        for message in [
            "docs: update guide",
            "revert: undo docs",
            "unconventional change",
            "feat!!: malformed",
        ] {
            let repo = Repo::new("0.1.0");
            repo.git(&["tag", "v0.1.0"]);
            repo.commit(message);
            assert!(plan_at(&repo.0, None).is_err(), "{message}");
            assert_eq!(repo.git(&["tag"]), "v0.1.0");
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct RepoState {
        head: String,
        refs: String,
        status: String,
        release_files: Vec<Vec<u8>>,
    }
    fn state(repo: &Repo) -> RepoState {
        RepoState {
            head: repo.git(&["rev-parse", "HEAD"]),
            refs: repo.git(&["show-ref"]),
            status: repo.git(&["status", "--porcelain"]),
            release_files: RELEASE_FILES
                .iter()
                .map(|path| fs::read(repo.0.join(path)).unwrap())
                .collect(),
        }
    }

    #[test]
    fn failed_prepare_checks_never_commit_or_tag() {
        for fail_at in [1, 2] {
            let repo = Repo::new("0.1.0");
            repo.git(&["tag", "v0.1.0"]);
            let before = state(&repo);
            let plan = Plan {
                version: Version(0, 2, 0),
                previous: Some("v0.1.0".into()),
                bootstrap: false,
            };
            let mut calls = 0;
            let result = prepare_at(
                &repo.0,
                &plan,
                || {
                    calls += 1;
                    if calls == 1 {
                        assert_eq!(state(&repo), before, "first checks must run before edits");
                    } else {
                        verify_release_at(&repo.0, "v0.2.0").unwrap();
                        assert!(
                            fs::read_to_string(repo.0.join("Cargo.lock"))
                                .unwrap()
                                .contains("version = \"0.2.0\"")
                        );
                    }
                    if calls == fail_at {
                        Err(format!("injected check failure {fail_at}").into())
                    } else {
                        Ok(())
                    }
                },
                |_, _| Ok("# Changelog\n\n## [0.2.0]\n\nFeatures.\n".into()),
            );
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("injected check failure")
            );
            assert_eq!(calls, fail_at);
            let after = state(&repo);
            assert_eq!(
                after.head, before.head,
                "failed checks must never create a commit"
            );
            assert_eq!(
                after.refs, before.refs,
                "failed checks must never create/change tags"
            );
            assert!(
                repo.git(&["diff", "--cached", "--name-only"]).is_empty(),
                "failed checks must never stage files"
            );
            if fail_at == 1 {
                assert_eq!(
                    after, before,
                    "first-gate failure must leave every file unchanged"
                );
            } else {
                assert!(
                    !after.status.is_empty(),
                    "second-gate failure leaves reviewable edits"
                );
                assert_eq!(current_version_at(&repo.0).unwrap(), "0.2.0");
            }
        }
    }

    #[test]
    fn successful_bootstrap_excludes_private_history_and_tags_only_explicitly() {
        let repo = Repo::new("0.1.0");
        repo.commit("PRIVATE_COMMIT_SENTINEL: machine-specific development notes");
        let count_before = repo
            .git(&["rev-list", "--count", "HEAD"])
            .parse::<usize>()
            .unwrap();
        let plan = plan_at(&repo.0, None).unwrap();
        let mut checks = 0;
        prepare_at(
            &repo.0,
            &plan,
            || {
                checks += 1;
                Ok(())
            },
            changelog_at,
        )
        .unwrap();
        assert_eq!(checks, 2);
        assert_eq!(
            repo.git(&["rev-list", "--count", "HEAD"])
                .parse::<usize>()
                .unwrap(),
            count_before + 1
        );
        assert_eq!(
            repo.git(&["log", "-1", "--format=%s"]),
            "chore(release): v0.1.0"
        );
        let changelog = fs::read_to_string(repo.0.join("CHANGELOG.md")).unwrap();
        assert!(changelog.contains("Initial public release"));
        assert!(!changelog.contains("PRIVATE_COMMIT_SENTINEL"));
        assert!(!changelog.contains("PRIVATE_CHANGELOG_SENTINEL"));
        assert!(!changelog.contains("private legacy history"));
        assert!(
            repo.git(&["tag"]).is_empty(),
            "prepare must never create a tag"
        );
        assert!(repo.git(&["status", "--porcelain"]).is_empty());
        let head = repo.git(&["rev-parse", "HEAD"]);
        tag_at(&repo.0).unwrap();
        assert_eq!(repo.git(&["cat-file", "-t", "v0.1.0"]), "tag");
        assert_eq!(repo.git(&["rev-parse", "v0.1.0^{}"]), head);
        assert_eq!(repo.git(&["rev-parse", "HEAD"]), head);
    }

    #[test]
    fn tag_rejections_preserve_head_refs_and_all_files() {
        for rejection in [
            "wrong subject",
            "missing heading",
            "heading suffix",
            "dirty",
            "duplicate",
        ] {
            let repo = Repo::new("0.1.0");
            let changelog = match rejection {
                "missing heading" => "# Changelog\n\n## [0.0.9]\n",
                "heading suffix" => "# Changelog\n\n## [0.1.0]-garbage\n",
                _ => "# Changelog\n\n## [0.1.0]\n",
            };
            fs::write(repo.0.join("CHANGELOG.md"), changelog).unwrap();
            let subject = if rejection == "wrong subject" {
                "docs: manual notes"
            } else {
                "chore(release): v0.1.0"
            };
            repo.commit(subject);
            if rejection == "dirty" {
                fs::write(
                    repo.0.join("CHANGELOG.md"),
                    format!("{changelog}uncommitted notes\n"),
                )
                .unwrap();
            }
            if rejection == "duplicate" {
                repo.git(&["tag", "-a", "v0.1.0", "-m", "existing release"]);
            }
            let before = state(&repo);
            assert!(tag_at(&repo.0).is_err(), "tag should reject {rejection}");
            assert_eq!(
                state(&repo),
                before,
                "tag rejection changed repo for {rejection}"
            );
        }
    }
}
