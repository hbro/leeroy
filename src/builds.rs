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

/// Which build of a job to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildRef {
    /// The newest build; follows new builds on refresh.
    Latest,
    Number(u64),
}

/// Request path (relative to the Jenkins URL) for `which` build of a job.
///
/// `Latest` asks the *job* for its build numbers and last build in one go
/// (the numbers are needed to step between builds: they have gaps where
/// builds were deleted). `Number` asks for that one build.
///
/// Built from the full name rather than the job's `url` from the API: that
/// URL uses Jenkins' configured root, which behind a reverse proxy can be an
/// internal address the user can't (or shouldn't) reach directly.
pub fn build_path(full_name: &str, which: BuildRef) -> String {
    let job_path = job_path(full_name);
    match which {
        // `allBuilds` has every number; `builds` is the fallback where it's
        // not exported (and may be capped to recent ones).
        BuildRef::Latest => {
            format!("{job_path}api/json?tree=allBuilds[number],builds[number],lastBuild[{TREE}]")
        }
        BuildRef::Number(n) => format!("{job_path}{n}/api/json?tree={TREE}"),
    }
}

/// Request path for just a job's build numbers.
pub fn numbers_path(full_name: &str) -> String {
    format!(
        "{}api/json?tree=allBuilds[number],builds[number]",
        job_path(full_name)
    )
}

/// What a build fetch returns: the build (`None`: not found / never built)
/// and, for [`BuildRef::Latest`], all build numbers (ascending).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildPage {
    pub build: Option<Build>,
    pub numbers: Option<Vec<u64>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawJob {
    all_builds: Option<Vec<RawNumber>>,
    builds: Option<Vec<RawNumber>>,
    last_build: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct RawNumber {
    number: u64,
}

/// Parse the job-level answer for [`BuildRef::Latest`].
pub fn parse_job_builds(json: &str) -> Result<BuildPage, String> {
    let raw: RawJob =
        serde_json::from_str(json).map_err(|err| format!("unexpected job data: {err}"))?;
    let mut numbers: Vec<u64> = raw
        .all_builds
        .or(raw.builds)
        .unwrap_or_default()
        .into_iter()
        .map(|n| n.number)
        .collect();
    numbers.sort_unstable();
    numbers.dedup();
    let build = match raw.last_build {
        Some(value) if !value.is_null() => Some(parse_build(&value.to_string())?),
        _ => None,
    };
    // The last build is newer than the list if one started in between.
    if let Some(build) = &build
        && !numbers.contains(&build.number)
    {
        numbers.push(build.number);
        numbers.sort_unstable();
    }
    Ok(BuildPage {
        build,
        numbers: Some(numbers),
    })
}

/// `job/<a>/job/<b>/` for full name `a/b`, segments percent-encoded.
pub fn job_path(full_name: &str) -> String {
    full_name
        .split('/')
        .map(|segment| format!("job/{}/", encode_segment(segment)))
        .collect()
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
    /// The build being shown (or loaded).
    pub target: BuildRef,
    /// All build numbers, ascending; known after the first `Latest` fetch.
    pub numbers: Option<Vec<u64>>,
    pub load: BuildLoad,
    /// A reload is running while the current details stay visible.
    pub refreshing: bool,
    pub fetched_at: Option<std::time::Instant>,
    pub attempted_at: Option<std::time::Instant>,
    /// Lines scrolled down.
    pub scroll: u16,
    /// The view Esc returns to (Jobs or Builds).
    pub origin: crate::app::View,
    /// Largest useful `scroll`, recorded by the renderer (which knows the
    /// content and screen height) so scrolling can be clamped.
    pub max_scroll: std::cell::Cell<u16>,
}

impl BuildView {
    pub fn new(job: String) -> Self {
        Self {
            job,
            target: BuildRef::Latest,
            numbers: None,
            load: BuildLoad::Loading,
            refreshing: false,
            fetched_at: None,
            attempted_at: None,
            scroll: 0,
            origin: crate::app::View::Jobs,
            max_scroll: std::cell::Cell::new(0),
        }
    }

    pub fn fetch_in_flight(&self) -> bool {
        self.refreshing || self.load == BuildLoad::Loading
    }

    /// Number of the build shown (or being loaded), if known.
    pub fn current_number(&self) -> Option<u64> {
        match self.target {
            BuildRef::Number(n) => Some(n),
            BuildRef::Latest => match &self.load {
                BuildLoad::Loaded(Some(build)) => Some(build.number),
                _ => self.numbers.as_ref()?.last().copied(),
            },
        }
    }

    /// Where a step would go: `None` when there's nowhere to go (edge of the
    /// list, or numbers not known yet). The newest build is always `Latest`,
    /// so a view that reaches it follows new builds again.
    pub fn step(&self, step: BuildStep) -> Option<BuildRef> {
        let numbers = self.numbers.as_ref().filter(|n| !n.is_empty())?;
        let current = self.current_number();
        let newest = *numbers.last()?;
        let target = match step {
            BuildStep::Older => {
                let current = current?;
                *numbers.iter().rev().find(|&&n| n < current)?
            }
            BuildStep::Newer => {
                let current = current?;
                *numbers.iter().find(|&&n| n > current)?
            }
            BuildStep::First => *numbers.first()?,
            BuildStep::Last => newest,
        };
        let target = if target == newest {
            BuildRef::Latest
        } else {
            BuildRef::Number(target)
        };
        (target != self.target).then_some(target)
    }

    /// Position of the shown build, 1 = oldest: `(position, total)`.
    pub fn position(&self) -> Option<(usize, usize)> {
        let numbers = self.numbers.as_ref()?;
        let current = self.current_number()?;
        let index = numbers.iter().position(|&n| n == current)?;
        Some((index + 1, numbers.len()))
    }
}

/// A step through a job's builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildStep {
    Older,
    Newer,
    First,
    Last,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_for_a_numbered_build() {
        let path = build_path("team/svc", BuildRef::Number(41));
        assert!(
            path.starts_with("job/team/job/svc/41/api/json?tree=number,"),
            "{path}"
        );
    }

    #[test]
    fn job_builds_list_with_gaps_and_a_newer_last_build() {
        let json = r#"{"allBuilds": [{"number": 7}, {"number": 3}, {"number": 5}],
                       "builds": [{"number": 7}],
                       "lastBuild": {"number": 8, "result": null, "building": true,
                                     "timestamp": 0, "duration": 0}}"#;
        let page = parse_job_builds(json).unwrap();
        assert_eq!(page.numbers, Some(vec![3, 5, 7, 8]));
        assert_eq!(page.build.unwrap().number, 8);

        // Fallback to `builds`; never built.
        let page = parse_job_builds(r#"{"builds": [], "lastBuild": null}"#).unwrap();
        assert_eq!(
            page,
            BuildPage {
                build: None,
                numbers: Some(vec![])
            }
        );
    }

    fn view(numbers: &[u64], target: BuildRef) -> BuildView {
        BuildView {
            numbers: Some(numbers.to_vec()),
            target,
            ..BuildView::new("job".into())
        }
    }

    #[test]
    fn stepping_skips_gaps_and_returns_to_latest() {
        let latest = view(&[3, 5, 7, 8], BuildRef::Latest);
        assert_eq!(latest.current_number(), Some(8));
        assert_eq!(latest.step(BuildStep::Older), Some(BuildRef::Number(7)));
        assert_eq!(latest.step(BuildStep::Newer), None, "already newest");
        assert_eq!(latest.step(BuildStep::Last), None, "already latest");
        assert_eq!(latest.step(BuildStep::First), Some(BuildRef::Number(3)));
        assert_eq!(latest.position(), Some((4, 4)));

        let at5 = view(&[3, 5, 7, 8], BuildRef::Number(5));
        assert_eq!(at5.step(BuildStep::Older), Some(BuildRef::Number(3)));
        assert_eq!(at5.step(BuildStep::Newer), Some(BuildRef::Number(7)));
        assert_eq!(at5.position(), Some((2, 4)));
        let at7 = view(&[3, 5, 7, 8], BuildRef::Number(7));
        assert_eq!(
            at7.step(BuildStep::Newer),
            Some(BuildRef::Latest),
            "newest = Latest"
        );
        let at3 = view(&[3, 5, 7, 8], BuildRef::Number(3));
        assert_eq!(at3.step(BuildStep::Older), None, "oldest");
        assert_eq!(at3.step(BuildStep::First), None, "already first");
    }

    #[test]
    fn no_stepping_before_numbers_are_known() {
        let fresh = BuildView::new("job".into());
        assert_eq!(fresh.step(BuildStep::Older), None);
        assert_eq!(fresh.step(BuildStep::First), None);
        assert_eq!(view(&[], BuildRef::Latest).step(BuildStep::First), None);
    }

    #[test]
    fn path_from_full_name() {
        let path = build_path("team/my svc/main#1", BuildRef::Latest);
        assert!(
            path.starts_with("job/team/job/my%20svc/job/main%231/api/json?tree=allBuilds[number],"),
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
