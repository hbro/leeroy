//! A job's most recent build: model, Jenkins JSON parsing, request path. No IO.

use std::time::{Duration, SystemTime};

use serde::Deserialize;

use crate::jobs::JobStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Build {
    pub number: u64,
    /// Usually `#42`; can be customised in Jenkins.
    pub display_name: String,
    /// `None` while running (Jenkins reports no result yet).
    pub result: Option<JobStatus>,
    pub building: bool,
    /// Start time.
    pub started: SystemTime,
    /// Zero while running.
    pub duration: Duration,
    /// Jenkins' estimate from previous builds; `None` if unknown.
    pub estimated: Option<Duration>,
    pub description: Option<String>,
    /// e.g. "Started by user Hans", "Started by timer".
    pub causes: Vec<String>,
    pub parameters: Vec<(String, String)>,
    pub changes: Vec<Change>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Short commit id, if the SCM provides one.
    pub commit: Option<String>,
    pub message: String,
    pub author: Option<String>,
}

/// Request path (relative to the Jenkins URL) for a job's last build.
///
/// Built from the full name rather than the job's `url` from the API: that
/// URL uses Jenkins' configured root, which behind a reverse proxy can be an
/// internal address the user can't (or shouldn't) reach directly.
pub fn last_build_path(full_name: &str) -> String {
    let job_path: String = full_name
        .split('/')
        .map(|segment| format!("job/{}/", encode_segment(segment)))
        .collect();
    format!("{job_path}lastBuild/api/json?tree={TREE}")
}

/// Percent-encode one path segment (job names may contain spaces, `#`, ...).
fn encode_segment(segment: &str) -> String {
    segment
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Fields fetched for a build. Freestyle jobs have `changeSet`, pipelines
/// `changeSets`; Jenkins ignores whichever doesn't apply.
const TREE: &str = concat!(
    "number,displayName,result,building,timestamp,duration,estimatedDuration,description,",
    "actions[causes[shortDescription],parameters[name,value]],",
    "changeSet[items[commitId,msg,author[fullName]]],",
    "changeSets[items[commitId,msg,author[fullName]]]"
);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawBuild {
    number: u64,
    display_name: Option<String>,
    result: Option<String>,
    #[serde(default)]
    building: bool,
    /// Milliseconds since the epoch.
    #[serde(default)]
    timestamp: u64,
    #[serde(default)]
    duration: u64,
    estimated_duration: Option<i64>,
    description: Option<String>,
    #[serde(default)]
    actions: Vec<Option<RawAction>>,
    change_set: Option<RawChangeSet>,
    #[serde(default)]
    change_sets: Vec<RawChangeSet>,
}

#[derive(Deserialize, Default)]
struct RawAction {
    #[serde(default)]
    causes: Vec<RawCause>,
    #[serde(default)]
    parameters: Vec<RawParameter>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCause {
    short_description: Option<String>,
}

#[derive(Deserialize)]
struct RawParameter {
    name: String,
    /// Any JSON: strings, booleans, numbers; absent for password parameters.
    value: Option<serde_json::Value>,
}

#[derive(Deserialize, Default)]
struct RawChangeSet {
    #[serde(default)]
    items: Vec<RawChange>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawChange {
    commit_id: Option<String>,
    #[serde(default)]
    msg: String,
    author: Option<RawAuthor>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAuthor {
    full_name: Option<String>,
}

/// Jenkins result string -> status (`None` for unknown/absent).
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

pub fn parse_build(json: &str) -> Result<Build, String> {
    let raw: RawBuild =
        serde_json::from_str(json).map_err(|err| format!("unexpected build data: {err}"))?;
    let actions: Vec<RawAction> = raw.actions.into_iter().flatten().collect();
    let causes = actions
        .iter()
        .flat_map(|a| &a.causes)
        .filter_map(|c| c.short_description.clone())
        .collect();
    let parameters = actions
        .iter()
        .flat_map(|a| &a.parameters)
        .map(|p| {
            let value = match &p.value {
                Some(serde_json::Value::String(s)) => s.clone(),
                Some(serde_json::Value::Null) | None => "(hidden)".into(),
                Some(other) => other.to_string(),
            };
            (p.name.clone(), value)
        })
        .collect();
    let changes = raw
        .change_set
        .into_iter()
        .chain(raw.change_sets)
        .flat_map(|set| set.items)
        .map(|item| Change {
            commit: item.commit_id.map(|id| id.chars().take(8).collect()),
            message: item
                .msg
                .lines()
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned(),
            author: item.author.and_then(|a| a.full_name),
        })
        .collect();
    Ok(Build {
        number: raw.number,
        display_name: raw
            .display_name
            .unwrap_or_else(|| format!("#{}", raw.number)),
        result: parse_result(raw.result.as_deref()),
        building: raw.building,
        started: SystemTime::UNIX_EPOCH + Duration::from_millis(raw.timestamp),
        duration: Duration::from_millis(raw.duration),
        estimated: raw
            .estimated_duration
            .filter(|ms| *ms > 0)
            .map(|ms| Duration::from_millis(ms as u64)),
        description: raw.description.filter(|d| !d.trim().is_empty()),
        causes,
        parameters,
        changes,
    })
}

/// `45s`, `2m 13s`, `1h 05m`, `3d 04h`.
pub fn format_duration(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m {:02}s", s / 60, s % 60),
        3600..86_400 => format!("{}h {:02}m", s / 3600, s % 3600 / 60),
        _ => format!("{}d {:02}h", s / 86_400, s % 86_400 / 3600),
    }
}

/// Loading state of the build view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum BuildLoad {
    #[default]
    Loading,
    /// `None`: the job has never been built.
    Loaded(Option<Build>),
    Failed(String),
}

/// The build view for one job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildView {
    pub job: String,
    pub load: BuildLoad,
    /// A reload is running while the current details stay visible.
    pub refreshing: bool,
    pub fetched_at: Option<std::time::Instant>,
    pub attempted_at: Option<std::time::Instant>,
    /// Lines scrolled down.
    pub scroll: u16,
    /// Largest useful `scroll`, recorded by the renderer (which knows the
    /// content and screen height) so scrolling can be clamped.
    pub max_scroll: std::cell::Cell<u16>,
}

impl BuildView {
    pub fn new(job: String) -> Self {
        Self {
            job,
            load: BuildLoad::Loading,
            refreshing: false,
            fetched_at: None,
            attempted_at: None,
            scroll: 0,
            max_scroll: std::cell::Cell::new(0),
        }
    }

    pub fn fetch_in_flight(&self) -> bool {
        self.refreshing || self.load == BuildLoad::Loading
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_from_full_name() {
        let path = last_build_path("team/my svc/main#1");
        assert!(
            path.starts_with("job/team/job/my%20svc/job/main%231/lastBuild/api/json?tree="),
            "{path}"
        );
    }

    #[test]
    fn parses_a_pipeline_build() {
        let json = r##"{
            "_class": "org.jenkinsci.plugins.workflow.job.WorkflowRun",
            "number": 42, "displayName": "#42", "result": "FAILURE", "building": false,
            "timestamp": 1700000000000, "duration": 133000, "estimatedDuration": 120000,
            "description": "  ",
            "actions": [
                {"_class": "hudson.model.CauseAction",
                 "causes": [{"shortDescription": "Started by user Hans"}]},
                {},
                null,
                {"_class": "hudson.model.ParametersAction", "parameters": [
                    {"name": "ENV", "value": "prod"},
                    {"name": "DRY_RUN", "value": false},
                    {"name": "TOKEN"}
                ]}
            ],
            "changeSets": [{"items": [
                {"commitId": "0123456789abcdef", "msg": "Fix login\n\nlong body",
                 "author": {"fullName": "Alice"}}
            ]}]
        }"##;
        let build = parse_build(json).unwrap();
        assert_eq!(build.number, 42);
        assert_eq!(build.result, Some(JobStatus::Failed));
        assert_eq!(build.duration, Duration::from_secs(133));
        assert_eq!(build.estimated, Some(Duration::from_secs(120)));
        assert_eq!(build.description, None, "blank description dropped");
        assert_eq!(build.causes, ["Started by user Hans"]);
        assert_eq!(
            build.parameters,
            [
                ("ENV".to_owned(), "prod".to_owned()),
                ("DRY_RUN".to_owned(), "false".to_owned()),
                ("TOKEN".to_owned(), "(hidden)".to_owned()),
            ]
        );
        assert_eq!(
            build.changes,
            [Change {
                commit: Some("01234567".into()),
                message: "Fix login".into(),
                author: Some("Alice".into()),
            }]
        );
    }

    #[test]
    fn parses_a_running_freestyle_build() {
        let json = r#"{"number": 7, "result": null, "building": true,
            "timestamp": 1700000000000, "duration": 0, "estimatedDuration": -1,
            "changeSet": {"items": [{"msg": "tweak", "author": null}]}}"#;
        let build = parse_build(json).unwrap();
        assert_eq!(build.display_name, "#7");
        assert_eq!(build.result, None);
        assert!(build.building);
        assert_eq!(build.estimated, None, "-1 = unknown");
        assert_eq!(build.changes[0].author, None);
    }

    #[test]
    fn durations() {
        assert_eq!(format_duration(Duration::from_secs(45)), "45s");
        assert_eq!(format_duration(Duration::from_secs(133)), "2m 13s");
        assert_eq!(format_duration(Duration::from_secs(3900)), "1h 05m");
        assert_eq!(format_duration(Duration::from_secs(273_600)), "3d 04h");
    }
}
