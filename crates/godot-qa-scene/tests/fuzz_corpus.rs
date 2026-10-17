//! Deterministic fuzz/mutation harness for [`parse_scene`] (plus a round-trip
//! `diff` sanity check).
//!
//! Dependency-free by design (the workspace intentionally carries zero
//! dev-dependencies): no `proptest`/`quickcheck`. Instead this is a small,
//! fully seeded SplitMix64-driven mutation engine that chews on a corpus built
//! from every real `.tscn` under `examples/sample_project` plus a handful of
//! synthetic pathological seeds, asserting `parse_scene` never panics no
//! matter how mangled the input is (its documented contract — see the module
//! doc on `parse_scene` in `src/parser.rs`).
//!
//! Everything here is seeded from fixed constants, so a failure is always
//! reproducible from the `(file, seed, mutation index)` embedded in its
//! panic/assert message — just re-run this file; there is no non-determinism
//! (no `SystemTime`, no thread interleaving affecting outcomes).

use std::fs;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use godot_qa_scene::{diff, parse_scene};

// ---------------------------------------------------------------------------
// Deterministic PRNG
// ---------------------------------------------------------------------------
//
// A SplitMix64 generator. The multiplier/shift constants mirror the ones used
// by `crates/godot-qa-runtime/src/random.rs`'s `fill_fallback` (the same
// well-known SplitMix64 constants), kept in sync here only for style
// consistency — this PRNG is test-local and never touches real randomness.

const GOLDEN_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
const MIX_1: u64 = 0xBF58_476D_1CE4_E5B9;
const MIX_2: u64 = 0x94D0_49BB_1331_11EB;

/// Fixed base seed for the whole harness. Combined with the corpus-file index
/// and mutant index, this makes every mutant fully deterministic across runs
/// and machines.
const BASE_SEED: u64 = 0xC0FF_EE15_5EED_0001;

/// How many mutants to generate per corpus file. Tuned so the full harness
/// (37 corpus files x this count, each mutant applying 1-4 mutation ops) runs
/// in well under a second in `cargo test --release` and a couple of seconds
/// in debug — comfortably inside the ~10s budget.
const MUTANTS_PER_FILE: u64 = 200;

struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        SplitMix64 { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(GOLDEN_GAMMA);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(MIX_1);
        z = (z ^ (z >> 27)).wrapping_mul(MIX_2);
        z ^= z >> 31;
        z
    }

    /// Uniform-ish value in `0..bound`. `bound == 0` always yields `0`.
    fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            return 0;
        }
        (self.next_u64() % bound as u64) as usize
    }

    fn byte(&mut self) -> u8 {
        (self.next_u64() & 0xFF) as u8
    }
}

// ---------------------------------------------------------------------------
// Mutation engine
// ---------------------------------------------------------------------------

const MUTATION_KIND_COUNT: usize = 9;

const INJECT_SYMBOLS: [u8; 11] = [
    b'"', b'[', b']', b'{', b'}', b'(', b')', b'=', b';', b'\\', b'$',
];

fn mutation_name(kind: usize) -> &'static str {
    match kind {
        0 => "byte_flip",
        1 => "byte_insert",
        2 => "byte_delete",
        3 => "truncate",
        4 => "duplicate_line",
        5 => "delete_line",
        6 => "swap_lines",
        7 => "inject_symbol",
        8 => "corrupt_section_header",
        _ => unreachable!("mutation kind out of range: {kind}"),
    }
}

fn apply_mutation(bytes: &mut Vec<u8>, rng: &mut SplitMix64, kind: usize) {
    match kind {
        0 => byte_flip(bytes, rng),
        1 => byte_insert(bytes, rng),
        2 => byte_delete(bytes, rng),
        3 => truncate(bytes, rng),
        4 => duplicate_line(bytes, rng),
        5 => delete_line(bytes, rng),
        6 => swap_lines(bytes, rng),
        7 => inject_symbol(bytes, rng),
        8 => corrupt_section_header(bytes, rng),
        _ => unreachable!("mutation kind out of range: {kind}"),
    }
}

fn byte_flip(bytes: &mut [u8], rng: &mut SplitMix64) {
    if bytes.is_empty() {
        return;
    }
    let index = rng.below(bytes.len());
    bytes[index] = rng.byte();
}

fn byte_insert(bytes: &mut Vec<u8>, rng: &mut SplitMix64) {
    let index = rng.below(bytes.len() + 1);
    bytes.insert(index, rng.byte());
}

fn byte_delete(bytes: &mut Vec<u8>, rng: &mut SplitMix64) {
    if bytes.is_empty() {
        return;
    }
    let index = rng.below(bytes.len());
    bytes.remove(index);
}

fn truncate(bytes: &mut Vec<u8>, rng: &mut SplitMix64) {
    if bytes.is_empty() {
        return;
    }
    let index = rng.below(bytes.len());
    bytes.truncate(index);
}

fn split_lines(bytes: &[u8]) -> Vec<Vec<u8>> {
    bytes
        .split(|&byte| byte == b'\n')
        .map(|s| s.to_vec())
        .collect()
}

fn join_lines(lines: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            out.push(b'\n');
        }
        out.extend_from_slice(line);
    }
    out
}

fn duplicate_line(bytes: &mut Vec<u8>, rng: &mut SplitMix64) {
    let mut lines = split_lines(bytes);
    if lines.is_empty() {
        return;
    }
    let index = rng.below(lines.len());
    let line = lines[index].clone();
    lines.insert(index, line);
    *bytes = join_lines(&lines);
}

fn delete_line(bytes: &mut Vec<u8>, rng: &mut SplitMix64) {
    let mut lines = split_lines(bytes);
    if lines.is_empty() {
        return;
    }
    let index = rng.below(lines.len());
    lines.remove(index);
    *bytes = join_lines(&lines);
}

fn swap_lines(bytes: &mut Vec<u8>, rng: &mut SplitMix64) {
    let mut lines = split_lines(bytes);
    if lines.len() < 2 {
        return;
    }
    let a = rng.below(lines.len());
    let b = rng.below(lines.len());
    lines.swap(a, b);
    *bytes = join_lines(&lines);
}

fn inject_symbol(bytes: &mut Vec<u8>, rng: &mut SplitMix64) {
    let index = rng.below(bytes.len() + 1);
    let symbol = INJECT_SYMBOLS[rng.below(INJECT_SYMBOLS.len())];
    bytes.insert(index, symbol);
}

/// Find a `[` and corrupt the header it opens: drop the matching `]`,
/// duplicate the `[`, or inject a stray symbol right after it. Falls back to
/// inserting a lone `[` when the input has no bracket at all, so this
/// mutation kind always does *something*.
fn corrupt_section_header(bytes: &mut Vec<u8>, rng: &mut SplitMix64) {
    let bracket_positions: Vec<usize> = bytes
        .iter()
        .enumerate()
        .filter(|&(_, &byte)| byte == b'[')
        .map(|(index, _)| index)
        .collect();
    if bracket_positions.is_empty() {
        let index = rng.below(bytes.len() + 1);
        bytes.insert(index, b'[');
        return;
    }
    let pos = bracket_positions[rng.below(bracket_positions.len())];
    match rng.below(3) {
        0 => {
            if let Some(offset) = bytes[pos..].iter().position(|&byte| byte == b']') {
                bytes.remove(pos + offset);
            } else {
                bytes.remove(pos);
            }
        }
        1 => bytes.insert(pos, b'['),
        _ => {
            let symbol = INJECT_SYMBOLS[rng.below(INJECT_SYMBOLS.len())];
            bytes.insert(pos + 1, symbol);
        }
    }
}

/// Build one mutant from `source` for `(file_index, mutant_index)`: 1-4
/// mutation ops chosen and applied in sequence, each seeded off the same
/// per-mutant RNG stream so the whole thing is reproducible from the seed
/// alone. Returns the resulting bytes plus the list of `(op_index,
/// mutation_name)` applied, for reproducible failure messages.
fn build_mutant(source: &[u8], seed: u64) -> (Vec<u8>, Vec<(usize, &'static str)>) {
    let mut rng = SplitMix64::new(seed);
    let mut bytes = source.to_vec();
    let op_count = 1 + rng.below(4); // 1..=4 ops per mutant
    let mut applied = Vec::with_capacity(op_count);
    for op_index in 0..op_count {
        let kind = rng.below(MUTATION_KIND_COUNT);
        apply_mutation(&mut bytes, &mut rng, kind);
        applied.push((op_index, mutation_name(kind)));
    }
    (bytes, applied)
}

fn seed_for(file_index: usize, mutant_index: u64) -> u64 {
    // Mix indices through a couple of SplitMix64 rounds off BASE_SEED so
    // nearby (file_index, mutant_index) pairs don't produce correlated seeds.
    let mut rng = SplitMix64::new(
        BASE_SEED
            ^ (file_index as u64).wrapping_mul(0x9E37_79B9)
            ^ mutant_index.wrapping_mul(0xBF58_476D_1CE4_E5B9),
    );
    rng.next_u64()
}

// ---------------------------------------------------------------------------
// Corpus
// ---------------------------------------------------------------------------

struct CorpusFile {
    label: String,
    content: String,
}

fn sample_project_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/sample_project")
}

fn collect_tscn_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|error| panic!("read_dir {}: {error}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| panic!("dir entry in {}: {error}", dir.display()));
        let path = entry.path();
        if path.is_dir() {
            collect_tscn_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("tscn") {
            out.push(path);
        }
    }
}

/// Every `.tscn` checked in under `examples/sample_project`. Asserts we found
/// a healthy number of files so silent corpus loss (a bad path, an accidental
/// filter) fails loudly instead of quietly fuzzing an empty set.
fn real_corpus() -> Vec<CorpusFile> {
    let dir = sample_project_dir();
    let mut paths = Vec::new();
    collect_tscn_files(&dir, &mut paths);
    paths.sort();
    assert!(
        paths.len() >= 20,
        "expected >= 20 .tscn files under {}, found {} — corpus loss?",
        dir.display(),
        paths.len()
    );
    paths
        .into_iter()
        .map(|path| {
            let content = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
            let label = path
                .strip_prefix(&dir)
                .unwrap_or(&path)
                .display()
                .to_string();
            CorpusFile { label, content }
        })
        .collect()
}

/// Hand-written pathological seeds targeting parser edge cases the real
/// sample scenes don't exercise: unterminated strings/brackets, nested
/// quotes, a leading BOM, a huge single line, a lone `[`, embedded
/// control/binary-ish characters, an empty file, a comment-only file,
/// duplicated section headers, and mixed CRLF/LF line endings.
fn synthetic_seeds() -> Vec<CorpusFile> {
    vec![
        CorpusFile {
            label: "synthetic:unterminated_string".to_string(),
            content: "[gd_scene format=3]\n[node name=\"Root\" type=\"Node\"]\na = \"unterminated"
                .to_string(),
        },
        CorpusFile {
            label: "synthetic:unterminated_brackets".to_string(),
            content:
                "[gd_scene format=3]\n[node name=\"Root\" type=\"Node\"]\na = [1, 2, {\"k\": [3, 4"
                    .to_string(),
        },
        CorpusFile {
            label: "synthetic:nested_quotes".to_string(),
            content:
                "[gd_scene format=3]\n[node name=\"Root\" type=\"Node\"]\na = \"outer \\\"inner \\'more\\' text\\\" end\"\n"
                    .to_string(),
        },
        CorpusFile {
            label: "synthetic:bom".to_string(),
            content: "\u{feff}[gd_scene format=3]\n[node name=\"Root\" type=\"Node\"]\na = 1\n"
                .to_string(),
        },
        CorpusFile {
            label: "synthetic:huge_single_line".to_string(),
            content: "x".repeat(200_000),
        },
        CorpusFile {
            label: "synthetic:lone_bracket".to_string(),
            content: "[".to_string(),
        },
        CorpusFile {
            label: "synthetic:binary_ish_escapes".to_string(),
            content: format!(
                "[gd_scene format=3]\n[node name=\"Root\" type=\"Node\"]\na = \"{}{}{}{}{}\"\n",
                '\u{0}', '\u{1}', '\u{1f}', '\u{7f}', '\u{1f600}'
            ),
        },
        CorpusFile {
            label: "synthetic:empty".to_string(),
            content: String::new(),
        },
        CorpusFile {
            label: "synthetic:only_comments".to_string(),
            content: "; just a comment\n; another one\n   ; indented comment\n".to_string(),
        },
        CorpusFile {
            label: "synthetic:duplicated_section_headers".to_string(),
            content:
                "[gd_scene format=3]\n[node name=\"Root\" type=\"Node\"]\n[node name=\"Root\" type=\"Node\"]\na = 1\n[node name=\"Root\" type=\"Node\"]\nb = 2\n"
                    .to_string(),
        },
        CorpusFile {
            label: "synthetic:crlf_mix".to_string(),
            content: "[gd_scene format=3]\r\n[node name=\"Root\" type=\"Node\"]\na = 1\r\nb = 2\n[node name=\"Child\" parent=\".\"]\r\nc = 3\n"
                .to_string(),
        },
    ]
}

fn full_corpus() -> Vec<CorpusFile> {
    let mut corpus = real_corpus();
    corpus.extend(synthetic_seeds());
    corpus
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The mutation-fuzz invariant: no matter how a corpus file gets mangled,
/// `parse_scene` must return normally (never panic) — this is `parse_scene`'s
/// documented contract ("Parsing never panics", see `src/parser.rs`).
#[test]
fn parse_scene_never_panics_on_mutated_corpus() {
    let corpus = full_corpus();
    assert!(!corpus.is_empty(), "corpus must not be empty");

    let mut mutants_run: u64 = 0;
    for (file_index, file) in corpus.iter().enumerate() {
        let source = file.content.as_bytes();
        for mutant_index in 0..MUTANTS_PER_FILE {
            let seed = seed_for(file_index, mutant_index);
            let (mutated_bytes, applied) = build_mutant(source, seed);
            // `parse_scene` takes `&str`; sanitize any invalid UTF-8 the byte
            // mutations may have produced exactly like a real editor/tool
            // would before handing text to the parser.
            let mutated_text = String::from_utf8_lossy(&mutated_bytes).into_owned();

            let file_label = file.label.clone();
            let applied_desc: Vec<String> = applied
                .iter()
                .map(|(op_index, name)| format!("#{op_index}={name}"))
                .collect();
            let result = panic::catch_unwind(AssertUnwindSafe(|| parse_scene(&mutated_text)));
            if let Err(payload) = result {
                let message = panic_message(&payload);
                panic!(
                    "parse_scene panicked on a mutant (reproducible via file={file_label:?}, seed=0x{seed:016X}, mutant_index={mutant_index}, ops=[{}]): {message}",
                    applied_desc.join(", ")
                );
            }
            mutants_run += 1;
        }
    }

    let expected = corpus.len() as u64 * MUTANTS_PER_FILE;
    assert_eq!(
        mutants_run, expected,
        "did not run the expected number of mutants"
    );
}

fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

/// Round-trip property on the pristine (unmutated) real corpus: diffing a
/// freshly parsed scene against a second, independent parse of the exact same
/// text must be a no-op — empty `ops` and no `fell_back` reasons. This is
/// restricted to the real `examples/sample_project` scenes (not the synthetic
/// pathological seeds): several synthetic seeds are deliberately rootless or
/// error-laden, and `diff` correctly (by its own documented contract) falls
/// back to "scene has no root node" / "scene has parse errors" even when
/// comparing such a scene against itself — that is expected fallback
/// behavior, not a round-trip violation.
#[test]
fn diff_of_pristine_scene_against_itself_is_a_no_op() {
    let corpus = real_corpus();
    for file in &corpus {
        let left = parse_scene(&file.content);
        let right = parse_scene(&file.content);
        assert!(
            !left.has_errors(),
            "pristine corpus file {:?} produced parse errors: {:?}",
            file.label,
            left.diagnostics
        );
        let out = diff(&left, &right);
        assert!(
            out.ops.is_empty(),
            "diff({0:?}, {0:?}) produced non-empty ops: {1:?}",
            file.label,
            out.ops
        );
        assert!(
            out.fell_back.is_empty(),
            "diff({0:?}, {0:?}) fell back: {1:?}",
            file.label,
            out.fell_back
        );
    }
}
