//! The job list: model, Jenkins JSON parsing, filtering and selection. No IO.

use serde::Deserialize;

use crate::input::TextInput;

/// Last build result, from the job's Jenkins `color`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Success,
    Unstable,
    Failed,
    Aborted,
    NotBuilt,
    Disabled,
    Unknown,
}

impl JobStatus {
    pub fn label(self) -> &'static str {
        match self {
            JobStatus::Success => "success",
            JobStatus::Unstable => "unstable",
            JobStatus::Failed => "failed",
            JobStatus::Aborted => "aborted",
            JobStatus::NotBuilt => "not built",
            JobStatus::Disabled => "disabled",
            JobStatus::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    /// Path including folders, e.g. `team/service/main`.
    pub full_name: String,
    pub url: String,
    pub status: JobStatus,
    /// A build is running right now (`*_anime` colors).
    pub building: bool,
}

/// Parse a Jenkins `color` value: `blue`, `red_anime`, `disabled`, ...
fn parse_color(color: Option<&str>) -> (JobStatus, bool) {
    let Some(color) = color else {
        return (JobStatus::Unknown, false);
    };
    let (base, building) = match color.strip_suffix("_anime") {
        Some(base) => (base, true),
        None => (color, false),
    };
    let status = match base {
        "blue" | "green" => JobStatus::Success,
        "yellow" => JobStatus::Unstable,
        "red" => JobStatus::Failed,
        "aborted" => JobStatus::Aborted,
        "notbuilt" | "nobuilt" => JobStatus::NotBuilt,
        "disabled" | "grey" => JobStatus::Disabled,
        _ => JobStatus::Unknown,
    };
    (status, building)
}

/// How deep `tree=` descends into folders (multibranch projects count as one).
pub const FOLDER_DEPTH: usize = 6;

/// The `tree` query for `/api/json`: jobs, recursing into folders.
pub fn tree_query() -> String {
    let mut tree = String::from("jobs[name,fullName,url,color]");
    for _ in 1..FOLDER_DEPTH {
        tree = format!("jobs[name,fullName,url,color,{tree}]");
    }
    tree
}

#[derive(Debug, Deserialize)]
struct Node {
    #[serde(default)]
    jobs: Vec<Item>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    name: String,
    full_name: Option<String>,
    #[serde(default)]
    url: String,
    color: Option<String>,
    /// Present on folders and multibranch projects: they contain jobs
    /// instead of being one.
    jobs: Option<Vec<Item>>,
}

/// Flatten the `/api/json` job tree: folders are descended into, not listed.
/// Sorted by full name.
pub fn parse_jobs(json: &str) -> Result<Vec<Job>, String> {
    let root: Node =
        serde_json::from_str(json).map_err(|err| format!("unexpected job list: {err}"))?;
    let mut jobs = Vec::new();
    flatten(root.jobs, "", &mut jobs);
    jobs.sort_by(|a, b| a.full_name.cmp(&b.full_name));
    Ok(jobs)
}

fn flatten(items: Vec<Item>, prefix: &str, out: &mut Vec<Job>) {
    for item in items {
        let full_name = item
            .full_name
            .unwrap_or_else(|| format!("{prefix}{}", item.name));
        match item.jobs {
            Some(children) => flatten(children, &format!("{full_name}/"), out),
            None => {
                let (status, building) = parse_color(item.color.as_deref());
                out.push(Job {
                    full_name,
                    url: item.url,
                    status,
                    building,
                });
            }
        }
    }
}

/// Case-insensitive; every whitespace-separated term must occur in the name.
pub fn matches(job: &Job, filter: &str) -> bool {
    let name = job.full_name.to_lowercase();
    filter
        .split_whitespace()
        .all(|term| name.contains(&term.to_lowercase()))
}

/// Loading state of the job list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum JobsLoad {
    #[default]
    NotLoaded,
    Loading,
    Loaded(Vec<Job>),
    Failed(String),
}

/// The Jobs tab: loaded jobs, filter and selection.
#[derive(Debug, Default)]
pub struct JobsState {
    pub load: JobsLoad,
    /// Applied filter text (may be empty).
    pub filter: String,
    /// Filter input while `/` is active; its text is applied live.
    pub filter_input: Option<TextInput>,
    /// Index into [`Self::visible`].
    pub selected: usize,
    /// A reload is running while the current list stays visible.
    pub refreshing: bool,
    /// When the list was last loaded successfully.
    pub fetched_at: Option<std::time::Instant>,
    /// When the last fetch finished, successfully or not: auto-refresh
    /// waits an interval from here, so failures aren't retried in a loop.
    pub attempted_at: Option<std::time::Instant>,
}

impl JobsState {
    /// The filter currently in effect: the live input while typing.
    pub fn active_filter(&self) -> &str {
        self.filter_input
            .as_ref()
            .map_or(self.filter.as_str(), |input| input.value())
    }

    /// Jobs matching the active filter, in display order.
    pub fn visible(&self) -> Vec<&Job> {
        match &self.load {
            JobsLoad::Loaded(jobs) => {
                let filter = self.active_filter();
                jobs.iter().filter(|job| matches(job, filter)).collect()
            }
            _ => Vec::new(),
        }
    }

    /// A fetch is running (first load or refresh).
    pub fn fetch_in_flight(&self) -> bool {
        self.refreshing || self.load == JobsLoad::Loading
    }

    pub fn total(&self) -> usize {
        match &self.load {
            JobsLoad::Loaded(jobs) => jobs.len(),
            _ => 0,
        }
    }

    /// Move the selection by `delta` rows, clamped to the visible list.
    pub fn move_selection(&mut self, delta: isize) {
        let len = self.visible().len();
        if len == 0 {
            self.selected = 0;
            return;
        }
        let target = self.selected as isize + delta;
        self.selected = target.clamp(0, len as isize - 1) as usize;
    }

    /// Keep the selection inside the list after it changed (filter, reload).
    pub fn clamp_selection(&mut self) {
        self.move_selection(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(name: &str) -> Job {
        Job {
            full_name: name.into(),
            url: String::new(),
            status: JobStatus::Success,
            building: false,
        }
    }

    #[test]
    fn colors() {
        assert_eq!(parse_color(Some("blue")), (JobStatus::Success, false));
        assert_eq!(parse_color(Some("red_anime")), (JobStatus::Failed, true));
        assert_eq!(parse_color(Some("yellow")), (JobStatus::Unstable, false));
        assert_eq!(parse_color(Some("notbuilt")), (JobStatus::NotBuilt, false));
        assert_eq!(parse_color(Some("disabled")), (JobStatus::Disabled, false));
        assert_eq!(
            parse_color(Some("aborted_anime")),
            (JobStatus::Aborted, true)
        );
        assert_eq!(parse_color(Some("chartreuse")), (JobStatus::Unknown, false));
        assert_eq!(parse_color(None), (JobStatus::Unknown, false));
    }

    #[test]
    fn tree_query_nests_folders() {
        let tree = tree_query();
        assert_eq!(tree.matches("jobs[").count(), FOLDER_DEPTH);
        assert!(
            tree.starts_with("jobs[name,fullName,url,color,jobs["),
            "{tree}"
        );
    }

    #[test]
    fn folders_are_flattened_not_listed() {
        let json = r#"{"jobs": [
            {"_class": "hudson.model.FreeStyleProject", "name": "zeta", "fullName": "zeta",
             "url": "u/zeta", "color": "blue"},
            {"_class": "com.cloudbees.hudson.plugins.folder.Folder", "name": "team",
             "fullName": "team", "url": "u/team", "jobs": [
                {"name": "svc", "fullName": "team/svc", "url": "u/svc", "jobs": [
                    {"name": "main", "fullName": "team/svc/main", "url": "u/main",
                     "color": "red_anime"}
                ]},
                {"name": "lib", "url": "u/lib", "color": "notbuilt"}
            ]},
            {"name": "empty-folder", "fullName": "empty-folder", "url": "u/e", "jobs": []}
        ]}"#;
        let jobs = parse_jobs(json).unwrap();
        let names: Vec<&str> = jobs.iter().map(|j| j.full_name.as_str()).collect();
        // Sorted; folders (incl. empty ones) not listed; missing fullName derived.
        assert_eq!(names, ["team/lib", "team/svc/main", "zeta"]);
        assert_eq!(jobs[1].status, JobStatus::Failed);
        assert!(jobs[1].building);
    }

    #[test]
    fn bad_json_is_an_error() {
        assert!(parse_jobs("<html>").is_err());
    }

    #[test]
    fn filter_terms() {
        let j = job("Team/Service-API/main");
        assert!(matches(&j, ""));
        assert!(matches(&j, "api"));
        assert!(matches(&j, "team main"));
        assert!(!matches(&j, "team release"));
    }

    fn loaded(names: &[&str]) -> JobsState {
        JobsState {
            load: JobsLoad::Loaded(names.iter().map(|n| job(n)).collect()),
            ..Default::default()
        }
    }

    #[test]
    fn live_filter_and_selection_clamp() {
        let mut state = loaded(&["a/one", "a/two", "b/three"]);
        state.move_selection(10);
        assert_eq!(state.selected, 2);
        state.filter_input = Some(TextInput::new("a/"));
        assert_eq!(state.visible().len(), 2);
        state.clamp_selection();
        assert_eq!(state.selected, 1);
        state.move_selection(-5);
        assert_eq!(state.selected, 0);
        assert_eq!(state.total(), 3);
    }

    #[test]
    fn empty_list_selection() {
        let mut state = loaded(&[]);
        state.move_selection(1);
        assert_eq!(state.selected, 0);
    }
}
