use super::{Duration, Instant, ManifestTestEntry, Path, Value, fs};

pub(super) struct BatchDeadlineMonitor<'a> {
    progress_path: &'a Path,
    budgets: Vec<(&'a ManifestTestEntry, Option<Duration>)>,
    processed_lines: usize,
    active: Option<(usize, Instant)>,
    pub(super) timed_out_index: Option<usize>,
}

impl<'a> BatchDeadlineMonitor<'a> {
    pub(super) fn new(
        progress_path: &'a Path,
        budgets: Vec<(&'a ManifestTestEntry, Option<Duration>)>,
    ) -> Self {
        Self {
            progress_path,
            budgets,
            processed_lines: 0,
            active: None,
            timed_out_index: None,
        }
    }

    pub(super) fn poll(&mut self) -> Option<Duration> {
        let progress = fs::read_to_string(self.progress_path).unwrap_or_default();
        for line in progress.lines().skip(self.processed_lines) {
            // A killed or currently writing process may leave an incomplete final line.
            let Ok(event) = serde_json::from_str::<Value>(line) else {
                break;
            };
            self.processed_lines += 1;
            let Some(index) = self.budgets.iter().position(|(entry, _)| {
                event["id"] == entry.id
                    && event["kind"] == entry.kind
                    && event["target"] == entry.target
            }) else {
                continue;
            };
            match event["event"].as_str() {
                Some("start") => self.active = Some((index, Instant::now())),
                Some("finish") => {
                    // Account for tests that start and finish between process polls.
                    if let Some(budget) = self.budgets[index].1
                        && event["elapsed_ms"]
                            .as_u64()
                            .is_some_and(|ms| Duration::from_millis(ms) >= budget)
                    {
                        self.timed_out_index = Some(index);
                        return Some(budget);
                    }
                    if self.active.is_some_and(|(active, _)| active == index) {
                        self.active = None;
                    }
                }
                _ => {}
            }
        }
        let (index, started) = self.active?;
        let budget = self.budgets[index].1?;
        if started.elapsed() >= budget {
            self.timed_out_index = Some(index);
            return Some(budget);
        }
        None
    }
}
