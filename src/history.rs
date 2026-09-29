//! Build history across all jobs: model, fetching strategy, parsing. No IO.
//!
//! Jenkins has no "newest N builds overall" endpoint, but `tree` supports
//! ranges: one request asks every job (folders included) for its newest `N`
//! builds (`builds[...]{0,N}`). The newest `N` builds overall are always among
//! those, so merging by start time and keeping the first `N` is exact. The same
//! holds for any filter on the job name: the newest `N` matching builds are
//! among the matching jobs' newest `N`.

use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, SystemTime},
};

use serde::Deserialize;

use crate::{
    input::TextInput,
    jobs::{JobStatus, Terms},
};

/// Folder depth followed (as for the job list).
const FOLDER_DEPTH: usize = crate::jobs::FOLDER_DEPTH;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub job: String,
    pub number: u64,
    /// `None` while running.
    pub result: Option<JobStatus>,
    pub building: bool,
    pub started: SystemTime,
    pub duration: Duration,
}

/// `tree` query: every job's newest `limit` builds, recursing into folders.
pub fn tree_query(limit: usize) -> String {
    let builds = format!("builds[number,result,building,timestamp,duration]{{0,{limit}}}");
    let mut tree = format!("jobs[fullName,{builds}]");
    for _ in 1..FOLDER_DEPTH {
        tree = format!("jobs[fullName,{builds},{tree}]");
    }
    tree
}

#[derive(Deserialize)]
struct Node {
    #[serde(default)]
    jobs: Vec<Item>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    full_name: Option<String>,
    #[serde(default)]
    builds: Vec<RawBuild>,
    jobs: Option<Vec<Item>>,
}

#[derive(Deserialize)]
struct RawBuild {
    number: u64,
    result: Option<String>,
    #[serde(default)]
    building: bool,
    #[serde(default)]
    timestamp: u64,
    #[serde(default)]
    duration: u64,
}

fn parse_result(result: Option<&str>) -> Option<JobStatus> {
    Some(match result? {
        "SUCCESS" => JobStatus::Success,
        "UNSTABLE" => JobStatus::Unstable,
        "FAILURE" => JobStatus::Failed,
        "ABORTED" => JobStatus::Aborted,
        "NOT_BUILT" => JobStatus::NotBuilt,
        _ => JobStatus::Unknown,
    })
}

/// All builds in the answer, newest first (not yet truncated).
pub fn parse_history(json: &str) -> Result<Vec<HistoryEntry>, String> {
    let root: Node =
        serde_json::from_str(json).map_err(|err| format!("unexpected build history: {err}"))?;
    let mut entries = Vec::new();
    collect(root.jobs, &mut entries);
    // Newest first; ties broken by job and number for a stable order.
    entries.sort_by(|a, b| {
        b.started
            .cmp(&a.started)
            .then_with(|| a.job.cmp(&b.job))
            .then_with(|| b.number.cmp(&a.number))
    });
    Ok(entries)
}

fn collect(items: Vec<Item>, out: &mut Vec<HistoryEntry>) {
    for item in items {
        if let Some(children) = item.jobs {
            collect(children, out);
            continue;
        }
        let Some(job) = item.full_name else { continue };
        for build in item.builds {
            out.push(HistoryEntry {
                job: job.clone(),
                number: build.number,
                result: parse_result(build.result.as_deref()),
                building: build.building,
                started: SystemTime::UNIX_EPOCH + Duration::from_millis(build.timestamp),
                duration: Duration::from_millis(build.duration),
            });
        }
    }
}

/// What the filter matches: the job name or `#number` (all terms must
/// match), lowercased. Complete for these, since the fetch strategy
/// guarantees every matching job's newest builds are loaded.
fn filter_key(entry: &HistoryEntry) -> String {
    format!("{} #{}", entry.job, entry.number).to_lowercase()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum HistoryLoad {
    #[default]
    NotLoaded,
    Loading,
    Loaded,
    Failed(String),
}

/// The Builds tab.
#[derive(Debug, Default)]
pub struct HistoryState {
    pub load: HistoryLoad,
    /// Every build in the last answer, newest first.
    entries: Vec<HistoryEntry>,
    /// Per entry, its [`filter_key`]: made once per answer, not per keypress
    /// (a big instance has 100k entries).
    keys: Vec<String>,
    /// Indices of the entries matching a filter, with that filter: a
    /// keypress asks several times for the same answer.
    matched: RefCell<Option<(String, Rc<[usize]>)>>,
    /// Builds per job asked for in the last successful fetch (and so the
    /// number of rows that are guaranteed correct).
    pub limit: usize,
    pub filter: String,
    pub filter_input: Option<TextInput>,
    pub selected: usize,
    pub refreshing: bool,
    pub fetched_at: Option<std::time::Instant>,
    pub attempted_at: Option<std::time::Instant>,
}

impl HistoryState {
    /// Store a fetched answer (made with `limit` builds per job).
    pub fn set_entries(&mut self, entries: Vec<HistoryEntry>, limit: usize) {
        self.keys = entries.iter().map(filter_key).collect();
        self.matched.take();
        self.entries = entries;
        self.limit = limit;
    }

    pub fn active_filter(&self) -> &str {
        self.filter_input
            .as_ref()
            .map_or(self.filter.as_str(), |input| input.value())
    }

    /// Indices of the entries matching the active filter, newest first.
    fn matching(&self) -> Rc<[usize]> {
        let filter = self.active_filter();
        let mut matched = self.matched.borrow_mut();
        if let Some((cached, indices)) = &*matched
            && cached == filter
        {
            return indices.clone();
        }
        let terms = Terms::new(filter);
        let indices: Rc<[usize]> = (0..self.keys.len())
            .filter(|&i| terms.matches(&self.keys[i]))
            .collect();
        *matched = Some((filter.to_owned(), indices.clone()));
        indices
    }

    /// Rows to show: the newest `limit` matching builds (beyond that the
    /// merged order could have gaps).
    pub fn visible(&self) -> Vec<&HistoryEntry> {
        self.matching()
            .iter()
            .take(self.limit)
            .map(|&i| &self.entries[i])
            .collect()
    }

    /// More matching builds exist than are shown: loading more (a larger
    /// `limit`) would show them.
    pub fn has_more(&self) -> bool {
        self.matching().len() > self.limit
    }

    pub fn fetch_in_flight(&self) -> bool {
        self.refreshing || self.load == HistoryLoad::Loading
    }

    pub fn move_selection(&mut self, delta: isize) {
        let len = self.visible().len();
        if len == 0 {
            self.selected = 0;
            return;
        }
        self.selected = (self.selected as isize + delta).clamp(0, len as isize - 1) as usize;
    }

    pub fn clamp_selection(&mut self) {
        self.move_selection(0);
    }

    /// The selection is on the last row shown.
    pub fn at_end(&self) -> bool {
        let len = self.visible().len();
        len > 0 && self.selected + 1 >= len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_asks_each_job_for_a_range_of_builds() {
        let tree = tree_query(25);
        assert!(tree.starts_with("jobs[fullName,builds[number,"), "{tree}");
        assert_eq!(tree.matches("{0,25}").count(), FOLDER_DEPTH);
    }

    fn json() -> &'static str {
        r#"{"jobs": [
            {"fullName": "a", "builds": [
                {"number": 3, "result": null, "building": true, "timestamp": 3000, "duration": 0},
                {"number": 2, "result": "SUCCESS", "timestamp": 1000, "duration": 5000}
            ]},
            {"fullName": "folder", "jobs": [
                {"fullName": "folder/b", "builds": [
                    {"number": 9, "result": "FAILURE", "timestamp": 2000, "duration": 1000}
                ]},
                {"fullName": "folder/never", "builds": []}
            ]}
        ]}"#
    }

    #[test]
    fn merged_newest_first_across_folders() {
        let entries = parse_history(json()).unwrap();
        let ids: Vec<(String, u64)> = entries.iter().map(|e| (e.job.clone(), e.number)).collect();
        assert_eq!(
            ids,
            [("a".into(), 3), ("folder/b".into(), 9), ("a".into(), 2)]
        );
        assert!(entries[0].building);
        assert_eq!(entries[1].result, Some(JobStatus::Failed));
    }

    fn state(limit: usize) -> HistoryState {
        let mut state = HistoryState::default();
        state.set_entries(parse_history(json()).unwrap(), limit);
        state
    }

    #[test]
    fn only_the_guaranteed_rows_are_shown() {
        let state = state(2);
        assert_eq!(state.visible().len(), 2);
        assert!(state.has_more());
        assert!(!self::state(5).has_more());
    }

    #[test]
    fn filter_on_job_and_number() {
        let mut state = state(10);
        state.filter = "folder".into();
        assert_eq!(state.visible().len(), 1);
        state.filter = "a #2".into();
        assert_eq!(state.visible()[0].number, 2);
        state.filter_input = Some(TextInput::new("#9"));
        assert_eq!(state.visible()[0].job, "folder/b", "live input wins");
        // Matches are remembered per filter, but not across answers.
        let mut newer = parse_history(json()).unwrap();
        newer.retain(|e| e.job != "folder/b");
        state.set_entries(newer, 10);
        assert!(state.visible().is_empty(), "a new answer, matched again");
        state.filter_input = Some(TextInput::new("A"));
        assert_eq!(state.visible().len(), 2, "case-insensitive");
    }

    #[test]
    fn selection() {
        let mut state = state(10);
        state.move_selection(10);
        assert_eq!(state.selected, 2);
        assert!(state.at_end());
        state.filter = "folder".into();
        state.clamp_selection();
        assert_eq!(state.selected, 0);
    }
}
