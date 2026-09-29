//! Pipelines and Runs tabs and the run view: which builds triggered which.
//! Model, fetching strategy, parsing, pipelines and their runs. No IO.
//!
//! A *pipeline* is a job whose own builds (started by nobody upstream)
//! trigger other jobs; a *run* is one such build plus every build that
//! followed from it. Its *parts* are the jobs it can reach.
//!
//! Jenkins records job relations in two places, and neither is complete:
//! - the static dependency graph (`upstreamProjects` / `downstreamProjects`),
//!   filled by freestyle "build other projects" and `upstream()` triggers,
//!   but empty for a Pipeline's `build job:` step;
//! - each triggered build's `UpstreamCause` (upstream job + build number),
//!   recorded for both kinds.
//!
//! One request asks for both: every job's static relations and its newest
//! [`RUN_BUILDS`] builds with their causes. Runs link builds through the
//! causes; the relations (union of both) give a pipeline's parts, which
//! decide whether a run is complete (see [`run_status`]).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{Duration, SystemTime};

use serde::Deserialize;

use crate::{input::TextInput, jobs::JobStatus};

/// Builds per job looked at for causes: how far back runs (and observed
/// edges) reach.
pub const RUN_BUILDS: usize = 20;

const FOLDER_DEPTH: usize = crate::jobs::FOLDER_DEPTH;

/// `tree` query: jobs (recursing into folders) with their static relations
/// and newest `limit` builds' upstream causes.
pub fn tree_query(limit: usize) -> String {
    let fields = format!(
        "fullName,color,upstreamProjects[fullName],downstreamProjects[fullName],\
         builds[number,result,building,timestamp,duration,\
         actions[causes[upstreamProject,upstreamBuild]]]{{0,{limit}}}"
    );
    let mut tree = format!("jobs[{fields}]");
    for _ in 1..FOLDER_DEPTH {
        tree = format!("jobs[{fields},{tree}]");
    }
    tree
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineJob {
    pub name: String,
    pub status: JobStatus,
    pub building: bool,
}

/// A build, linked to the build that triggered it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunBuild {
    pub job: String,
    pub number: u64,
    /// `None` while running.
    pub result: Option<JobStatus>,
    pub building: bool,
    pub started: SystemTime,
    pub duration: Duration,
    /// The upstream build that triggered this one: `(job, number)`.
    pub parent: Option<(String, u64)>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PipelineData {
    /// Every job, sorted by name.
    pub jobs: Vec<PipelineJob>,
    /// `(upstream, downstream)`, both known jobs, sorted.
    pub edges: Vec<(String, String)>,
    /// The part of `edges` from the job configuration (Jenkins' dependency
    /// graph), which includes manual steps that haven't run.
    pub static_edges: Vec<(String, String)>,
    pub builds: Vec<RunBuild>,
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
    color: Option<String>,
    #[serde(default)]
    upstream_projects: Vec<NameRef>,
    #[serde(default)]
    downstream_projects: Vec<NameRef>,
    #[serde(default)]
    builds: Vec<RawBuild>,
    jobs: Option<Vec<Item>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NameRef {
    full_name: Option<String>,
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
    #[serde(default)]
    actions: Vec<Option<RawAction>>,
}

#[derive(Deserialize)]
struct RawAction {
    #[serde(default)]
    causes: Vec<RawCause>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCause {
    upstream_project: Option<String>,
    upstream_build: Option<u64>,
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

pub fn parse(json: &str) -> Result<PipelineData, String> {
    let root: Node =
        serde_json::from_str(json).map_err(|err| format!("unexpected job data: {err}"))?;
    let mut jobs = Vec::new();
    let mut edges = BTreeSet::new();
    let mut builds = Vec::new();
    collect(root.jobs, &mut jobs, &mut edges, &mut builds);
    jobs.sort_by(|a: &PipelineJob, b| a.name.cmp(&b.name));
    let known: BTreeSet<&str> = jobs.iter().map(|j| j.name.as_str()).collect();
    let keep = |(a, b): &(String, String)| {
        a != b && known.contains(a.as_str()) && known.contains(b.as_str())
    };
    let static_edges: Vec<(String, String)> = edges.iter().filter(|e| keep(e)).cloned().collect();
    // Observed edges: whoever triggered a build is upstream of its job.
    for build in &builds {
        if let Some((upstream, _)) = &build.parent {
            edges.insert((upstream.clone(), build.job.clone()));
        }
    }
    // Relations to jobs outside what we can see (deleted, too deep) are dropped.
    let edges = edges.into_iter().filter(|e| keep(e)).collect();
    Ok(PipelineData {
        jobs,
        edges,
        static_edges,
        builds,
    })
}

fn collect(
    items: Vec<Item>,
    jobs: &mut Vec<PipelineJob>,
    edges: &mut BTreeSet<(String, String)>,
    builds: &mut Vec<RunBuild>,
) {
    for item in items {
        if let Some(children) = item.jobs {
            collect(children, jobs, edges, builds);
            continue;
        }
        let Some(name) = item.full_name else { continue };
        let (status, building) = crate::jobs::parse_color(item.color.as_deref());
        for up in item
            .upstream_projects
            .into_iter()
            .filter_map(|r| r.full_name)
        {
            edges.insert((up, name.clone()));
        }
        for down in item
            .downstream_projects
            .into_iter()
            .filter_map(|r| r.full_name)
        {
            edges.insert((name.clone(), down));
        }
        for raw in item.builds {
            let parent = raw
                .actions
                .iter()
                .flatten()
                .flat_map(|a| &a.causes)
                .find_map(|c| Some((c.upstream_project.clone()?, c.upstream_build?)));
            builds.push(RunBuild {
                job: name.clone(),
                number: raw.number,
                result: parse_result(raw.result.as_deref()),
                building: raw.building,
                started: SystemTime::UNIX_EPOCH + Duration::from_millis(raw.timestamp),
                duration: Duration::from_millis(raw.duration),
                parent,
            });
        }
        jobs.push(PipelineJob {
            name,
            status,
            building,
        });
    }
}

/// Every whitespace-separated term occurs in `text` (case-insensitive).
fn matches(text: &str, filter: &str) -> bool {
    let text = text.to_lowercase();
    filter
        .split_whitespace()
        .all(|term| text.contains(&term.to_lowercase()))
}

/// Who triggered whom, among the loaded builds.
struct Links {
    /// Per build: the builds it triggered, oldest first.
    children: Vec<Vec<usize>>,
}

fn links(data: &PipelineData) -> Links {
    let index: HashMap<(&str, u64), usize> = data
        .builds
        .iter()
        .enumerate()
        .map(|(i, b)| ((b.job.as_str(), b.number), i))
        .collect();
    let mut children = vec![Vec::new(); data.builds.len()];
    for (i, build) in data.builds.iter().enumerate() {
        if let Some(&parent) = build
            .parent
            .as_ref()
            .and_then(|(job, n)| index.get(&(job.as_str(), *n)))
        {
            children[parent].push(i);
        }
    }
    for list in &mut children {
        list.sort_by(|&a, &b| {
            let (a, b) = (&data.builds[a], &data.builds[b]);
            a.started
                .cmp(&b.started)
                .then_with(|| a.job.cmp(&b.job))
                .then_with(|| a.number.cmp(&b.number))
        });
    }
    Links { children }
}

/// One run of a pipeline: an untriggered build of its first job and every
/// build that followed from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    /// Indices into [`PipelineData::builds`], depth first (tree order),
    /// starting with the build that started the run.
    pub builds: Vec<usize>,
    /// Per entry of `builds`: the entry that triggered it (`None` for the first).
    pub parents: Vec<Option<usize>>,
    /// Per entry of `builds`: tree branches before it (`""`, `"├─ "`, `"│  └─ "`).
    pub prefixes: Vec<String>,
    /// [`run_status`], worked out once when the pipelines are built.
    pub status: RunStatus,
    pub running: bool,
}

impl Run {
    /// The build that started the run.
    pub fn first(&self) -> usize {
        self.builds[0]
    }
}

fn collect_run(links: &Links, root: usize) -> Run {
    let mut run = Run {
        builds: vec![root],
        parents: vec![None],
        prefixes: vec![String::new()],
        // Filled in by `pipelines`, which knows the pipeline's parts.
        status: RunStatus::Partial,
        running: false,
    };
    fn walk(links: &Links, node: usize, at: usize, indent: &str, run: &mut Run) {
        let kids = &links.children[node];
        for (i, &child) in kids.iter().enumerate() {
            let last = i + 1 == kids.len();
            run.builds.push(child);
            run.parents.push(Some(at));
            run.prefixes
                .push(format!("{indent}{}", if last { "└─ " } else { "├─ " }));
            let deeper = format!("{indent}{}", if last { "   " } else { "│  " });
            walk(links, child, run.builds.len() - 1, &deeper, run);
        }
    }
    walk(links, root, 0, "", &mut run);
    run
}

/// How a run went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RunStatus {
    /// Every part of the pipeline succeeded.
    Success,
    /// Nothing went wrong, but not every part has succeeded (yet): still
    /// running, or some part wasn't reached.
    Partial,
    Aborted,
    Unstable,
    Failure,
}

impl RunStatus {
    pub fn label(self) -> &'static str {
        match self {
            RunStatus::Success => "success",
            RunStatus::Partial => "partial",
            RunStatus::Aborted => "aborted",
            RunStatus::Unstable => "unstable",
            RunStatus::Failure => "failure",
        }
    }
}

/// A job whose own (untriggered) builds start runs of other jobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pipeline {
    /// The job that starts its runs.
    pub first_job: String,
    /// Common prefix of its jobs when there is a (unique) one, else `first_job`.
    pub name: String,
    /// Every job it can reach (its parts), sorted; `first_job` included.
    pub parts: Vec<String>,
    /// Newest first.
    pub runs: Vec<Run>,
}

/// Separators a name prefix may end at.
const SEPARATORS: [char; 4] = ['/', '-', '_', '.'];

/// The longest common prefix of `names` that ends at a word boundary, without
/// trailing separators: `shop/build`, `shop/deploy` → `shop`. `None` if empty.
fn common_prefix(names: &[String]) -> Option<String> {
    let first = names.first()?;
    let mut len = first.len();
    for name in &names[1..] {
        len = first
            .char_indices()
            .zip(name.chars())
            .take_while(|((_, a), b)| a == b)
            .map(|((i, a), _)| i + a.len_utf8())
            .last()
            .unwrap_or(0)
            .min(len);
    }
    // Only whole words: every name must continue with a separator (or end).
    let whole = |len: usize| {
        names.iter().all(|n| {
            n[len..]
                .chars()
                .next()
                .is_none_or(|c| SEPARATORS.contains(&c))
        })
    };
    let mut prefix = &first[..len];
    if !whole(len) {
        let cut = prefix.rfind(SEPARATORS).unwrap_or(0);
        prefix = &first[..cut];
    }
    let prefix = prefix.trim_end_matches(SEPARATORS);
    (!prefix.is_empty()).then(|| prefix.to_owned())
}

/// Every pipeline with its runs, sorted by name.
pub fn pipelines(data: &PipelineData) -> Vec<Pipeline> {
    let links = links(data);
    // Runs start with builds nobody triggered (no upstream cause at all: a
    // build whose trigger is just too old to be loaded is still triggered).
    // A job is a pipeline when such a build of it triggered something; then
    // all its untriggered builds are runs (a failed one triggers nothing).
    // Except a job the configuration says another job triggers: that's a
    // step of that pipeline even when someone starts it by hand.
    let configured_step: BTreeSet<&str> = data
        .static_edges
        .iter()
        .map(|(_, to)| to.as_str())
        .collect();
    let mut starts: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, build) in data.builds.iter().enumerate() {
        if build.parent.is_none()
            && !links.children[i].is_empty()
            && !configured_step.contains(build.job.as_str())
        {
            starts.entry(build.job.as_str()).or_default();
        }
    }
    for (i, build) in data.builds.iter().enumerate() {
        if build.parent.is_none()
            && let Some(list) = starts.get_mut(build.job.as_str())
        {
            list.push(i);
        }
    }

    let mut downstream: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (a, b) in &data.edges {
        downstream.entry(a.as_str()).or_default().push(b.as_str());
    }
    let mut result: Vec<Pipeline> = starts
        .into_iter()
        .map(|(job, mut roots)| {
            roots.sort_by_key(|&i| std::cmp::Reverse(data.builds[i].number));
            let mut parts = BTreeSet::from([job]);
            let mut queue = vec![job];
            while let Some(next) = queue.pop() {
                for &down in downstream.get(next).into_iter().flatten() {
                    if parts.insert(down) {
                        queue.push(down);
                    }
                }
            }
            let parts: Vec<String> = parts.into_iter().map(str::to_owned).collect();
            let mut pipeline = Pipeline {
                first_job: job.to_owned(),
                name: common_prefix(&parts).unwrap_or_else(|| job.to_owned()),
                parts,
                runs: roots.into_iter().map(|r| collect_run(&links, r)).collect(),
            };
            let statuses: Vec<(RunStatus, bool)> = pipeline
                .runs
                .iter()
                .map(|run| run_status(data, &pipeline, run))
                .collect();
            for (run, (status, running)) in pipeline.runs.iter_mut().zip(statuses) {
                run.status = status;
                run.running = running;
            }
            pipeline
        })
        .collect();
    // A name shared by two pipelines says nothing: use their first jobs.
    let mut counts: HashMap<String, usize> = HashMap::new();
    for p in &result {
        *counts.entry(p.name.clone()).or_default() += 1;
    }
    for p in &mut result {
        if counts[&p.name] > 1 {
            p.name = p.first_job.clone();
        }
    }
    result.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.first_job.cmp(&b.first_job))
    });
    result
}

/// Parts of the pipeline without any build in the run (why a run whose
/// builds all succeeded is still partial), with whether each is disabled.
pub fn missing_parts(data: &PipelineData, pipeline: &Pipeline, run: &Run) -> Vec<(String, bool)> {
    let present: BTreeSet<&str> = run
        .builds
        .iter()
        .map(|&b| data.builds[b].job.as_str())
        .collect();
    pipeline
        .parts
        .iter()
        .filter(|part| !present.contains(part.as_str()))
        .map(|part| {
            let disabled = data
                .jobs
                .iter()
                .any(|j| j.name == *part && j.status == JobStatus::Disabled);
            (part.clone(), disabled)
        })
        .collect()
}

/// One row of a run's tree: a build, or a part of the pipeline the run
/// never reached (shown where it would have run).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeItem {
    /// Index into [`Run::builds`].
    Build(usize),
    Missing {
        job: String,
        disabled: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    pub item: TreeItem,
    /// Tree branches before it (`""`, `"├─ "`, `"│  └─ "`).
    pub prefix: String,
    /// The row it hangs under.
    pub parent: Option<usize>,
}

/// A run as tree rows: its builds, plus the parts it never reached, each
/// under a build (or missing part) whose job triggers it — failing that,
/// under the first build.
pub fn run_tree(data: &PipelineData, pipeline: &Pipeline, run: &Run) -> Vec<TreeRow> {
    let n = run.builds.len();
    let missing = missing_parts(data, pipeline, run);
    let job_of = |node: usize| -> &str {
        if node < n {
            &data.builds[run.builds[node]].job
        } else {
            &missing[node - n].0
        }
    };
    let triggers = |from: &str, to: &str| data.edges.iter().any(|(a, b)| a == from && b == to);
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); n + missing.len()];
    for (i, parent) in run.parents.iter().enumerate() {
        if let Some(p) = parent {
            children[*p].push(i);
        }
    }
    // Place missing parts; one under another may need a few rounds.
    let mut placed = vec![false; missing.len()];
    loop {
        let mut progress = false;
        for m in 0..missing.len() {
            if placed[m] {
                continue;
            }
            let job = missing[m].0.as_str();
            let parent = (0..n).find(|&b| triggers(job_of(b), job)).or_else(|| {
                (0..missing.len())
                    .find(|&o| placed[o] && triggers(&missing[o].0, job))
                    .map(|o| n + o)
            });
            if let Some(parent) = parent {
                children[parent].push(n + m);
                placed[m] = true;
                progress = true;
            }
        }
        if !progress {
            break;
        }
    }
    for (m, _) in placed.iter().enumerate().filter(|(_, placed)| !**placed) {
        if n > 0 {
            children[0].push(n + m);
        }
    }

    let mut rows = Vec::new();
    fn walk(
        node: usize,
        prefix: String,
        indent: &str,
        parent: Option<usize>,
        children: &[Vec<usize>],
        item: &dyn Fn(usize) -> TreeItem,
        rows: &mut Vec<TreeRow>,
    ) {
        rows.push(TreeRow {
            item: item(node),
            prefix,
            parent,
        });
        let row = rows.len() - 1;
        let kids = &children[node];
        for (i, &child) in kids.iter().enumerate() {
            let last = i + 1 == kids.len();
            let branch = format!("{indent}{}", if last { "└─ " } else { "├─ " });
            let deeper = format!("{indent}{}", if last { "   " } else { "│  " });
            walk(child, branch, &deeper, Some(row), children, item, rows);
        }
    }
    let item = |node: usize| {
        if node < n {
            TreeItem::Build(node)
        } else {
            let (job, disabled) = missing[node - n].clone();
            TreeItem::Missing { job, disabled }
        }
    };
    if n > 0 {
        walk(0, String::new(), "", None, &children, &item, &mut rows);
    }
    rows
}

/// A run's status, and whether a build of it is still running.
pub fn run_status(data: &PipelineData, pipeline: &Pipeline, run: &Run) -> (RunStatus, bool) {
    let builds: Vec<&RunBuild> = run.builds.iter().map(|&i| &data.builds[i]).collect();
    let running = builds.iter().any(|b| b.building);
    let worst = builds
        .iter()
        .filter_map(|b| match b.result {
            Some(JobStatus::Failed) => Some(RunStatus::Failure),
            Some(JobStatus::Unstable) => Some(RunStatus::Unstable),
            Some(JobStatus::Aborted) => Some(RunStatus::Aborted),
            _ => None,
        })
        .max();
    let status = worst.unwrap_or_else(|| {
        let succeeded: BTreeSet<&str> = builds
            .iter()
            .filter(|b| b.result == Some(JobStatus::Success))
            .map(|b| b.job.as_str())
            .collect();
        if pipeline
            .parts
            .iter()
            .all(|part| succeeded.contains(part.as_str()))
        {
            RunStatus::Success
        } else {
            RunStatus::Partial
        }
    });
    (status, running)
}

/// When a run started, and when its last build finished (`None`: running).
pub fn run_span(data: &PipelineData, run: &Run) -> (SystemTime, Option<SystemTime>) {
    let builds = run.builds.iter().map(|&i| &data.builds[i]);
    let start = data.builds[run.first()].started;
    if builds.clone().any(|b| b.building) {
        return (start, None);
    }
    let end = builds
        .map(|b| b.started + b.duration)
        .max()
        .unwrap_or(start);
    (start, Some(end))
}

/// A manual step a run could take next: `job`, from `from_job` #`from_number`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Promotion {
    pub from_job: String,
    pub from_number: u64,
    pub job: String,
}

impl Promotion {
    pub fn label(&self) -> String {
        format!("{} #{} → {}", self.from_job, self.from_number, self.job)
    }
}

/// The run's available promotions: for every successfully finished build,
/// the jobs downstream of it in the job configuration (manual steps like the
/// Build Pipeline plugin's are declared there too) that it hasn't triggered.
pub fn promotions(data: &PipelineData, run: &Run) -> Vec<Promotion> {
    let mut out = Vec::new();
    for (i, &b) in run.builds.iter().enumerate() {
        let build = &data.builds[b];
        if build.building || build.result != Some(JobStatus::Success) {
            continue;
        }
        let triggered: BTreeSet<&str> = run
            .parents
            .iter()
            .enumerate()
            .filter(|(_, p)| **p == Some(i))
            .map(|(c, _)| data.builds[run.builds[c]].job.as_str())
            .collect();
        for (from, to) in &data.static_edges {
            if *from == build.job && !triggered.contains(to.as_str()) {
                out.push(Promotion {
                    from_job: build.job.clone(),
                    from_number: build.number,
                    job: to.clone(),
                });
            }
        }
    }
    out
}

/// A list tab's filter and selection.
#[derive(Debug, Default)]
pub struct ListState {
    pub filter: String,
    pub filter_input: Option<TextInput>,
    /// Index into the visible rows.
    pub selected: usize,
    /// First row on screen, recorded by the renderer, which only builds the
    /// rows that fit (these lists can have thousands).
    pub offset: std::cell::Cell<usize>,
}

impl ListState {
    pub fn active_filter(&self) -> &str {
        self.filter_input
            .as_ref()
            .map_or(self.filter.as_str(), |input| input.value())
    }

    pub fn move_selection(&mut self, delta: isize, len: usize) {
        self.selected = if len == 0 {
            0
        } else {
            (self.selected as isize + delta).clamp(0, len as isize - 1) as usize
        };
    }
}

/// Which run of a pipeline the run view shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunRef {
    /// The newest; follows new runs on refresh.
    Latest,
    /// The run started by this build number of the first job.
    Number(u64),
}

/// The run view: one run of one pipeline, as boxes or as a tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunView {
    /// Identifies the pipeline.
    pub first_job: String,
    pub target: RunRef,
    /// Tree (the default) instead of boxes.
    pub tree: bool,
    /// Selected build: index into [`Run::builds`].
    pub selected: usize,
    /// The view Esc returns to (Pipelines or Runs).
    pub origin: crate::app::View,
    /// Top-left of the visible part of the box drawing, recorded by the
    /// renderer, which scrolls to keep the selection in view.
    pub scroll: std::cell::Cell<(usize, usize)>,
}

impl RunView {
    pub fn new(first_job: String, target: RunRef, origin: crate::app::View) -> Self {
        Self {
            first_job,
            target,
            tree: true,
            selected: 0,
            origin,
            scroll: Default::default(),
        }
    }
}

/// Data shared by the Pipelines and Runs tabs (one fetch) plus their UI state.
#[derive(Debug, Default)]
pub struct PipelineState {
    pub load: PipelineLoad,
    /// The last fetch, and the pipelines worked out from it: rebuilding those
    /// on every keypress and frame made the lists sluggish on big instances.
    /// Only [`Self::set_data`] changes them, together.
    data: PipelineData,
    computed: Vec<Pipeline>,
    /// Every run as `(pipeline, run)`, newest first (for the Runs tab).
    run_order: Vec<(usize, usize)>,
    pub refreshing: bool,
    pub fetched_at: Option<std::time::Instant>,
    pub attempted_at: Option<std::time::Instant>,
    pub list: ListState,
    pub runs: ListState,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum PipelineLoad {
    #[default]
    NotLoaded,
    Loading,
    Loaded,
    Failed(String),
}

impl PipelineState {
    pub fn fetch_in_flight(&self) -> bool {
        self.refreshing || self.load == PipelineLoad::Loading
    }

    pub fn with_data(data: PipelineData) -> Self {
        let mut state = Self::default();
        state.set_data(data);
        state
    }

    /// Store a fetch and work out its pipelines (once).
    pub fn set_data(&mut self, data: PipelineData) {
        let computed = pipelines(&data);
        let mut order: Vec<(usize, usize)> = computed
            .iter()
            .enumerate()
            .flat_map(|(p, pipeline)| (0..pipeline.runs.len()).map(move |r| (p, r)))
            .collect();
        order.sort_by_key(|&(p, r)| {
            std::cmp::Reverse(data.builds[computed[p].runs[r].first()].started)
        });
        self.run_order = order;
        self.computed = computed;
        self.data = data;
    }

    pub fn data(&self) -> &PipelineData {
        &self.data
    }

    pub fn pipelines(&self) -> &[Pipeline] {
        &self.computed
    }

    /// Pipelines matching the Pipelines tab's filter (name or first job).
    pub fn visible_pipelines(&self) -> Vec<&Pipeline> {
        let filter = self.list.active_filter();
        self.computed
            .iter()
            .filter(|p| matches(&format!("{} {}", p.name, p.first_job), filter))
            .collect()
    }

    /// The Runs tab: `(pipeline, run)` indices into `pipelines`, newest
    /// first, matching its filter (pipeline name or any `job #number`).
    pub fn visible_runs(&self) -> Vec<(usize, usize)> {
        let filter = self.runs.active_filter();
        if filter.trim().is_empty() {
            return self.run_order.clone();
        }
        self.run_order
            .iter()
            .copied()
            .filter(|&(p, r)| {
                let pipeline = &self.computed[p];
                matches(&pipeline.name, filter)
                    || pipeline.runs[r].builds.iter().any(|&b| {
                        let b = &self.data.builds[b];
                        matches(&format!("{} #{}", b.job, b.number), filter)
                    })
            })
            .collect()
    }

    /// Number of the build that started `run`.
    pub fn run_number(&self, run: &Run) -> u64 {
        self.data.builds[run.first()].number
    }

    /// The pipeline and run index a run view shows, if loaded.
    pub fn find_run(&self, view: &RunView) -> Option<(&Pipeline, usize)> {
        let pipeline = self
            .computed
            .iter()
            .find(|p| p.first_job == view.first_job)?;
        let index = match view.target {
            RunRef::Latest => (!pipeline.runs.is_empty()).then_some(0)?,
            RunRef::Number(n) => pipeline.runs.iter().position(|r| self.run_number(r) == n)?,
        };
        Some((pipeline, index))
    }

    /// Where a step through a pipeline's runs goes (`None`: nowhere). The
    /// newest run is always `Latest`, so it follows new runs again.
    pub fn step_run(&self, view: &RunView, step: crate::builds::BuildStep) -> Option<RunRef> {
        use crate::builds::BuildStep;
        let (pipeline, index) = self.find_run(view)?;
        let last = pipeline.runs.len().checked_sub(1)?;
        let target = match step {
            BuildStep::Older => (index < last).then_some(index + 1)?,
            BuildStep::Newer => index.checked_sub(1)?,
            BuildStep::First => last,
            BuildStep::Last => 0,
        };
        let target = if target == 0 {
            RunRef::Latest
        } else {
            RunRef::Number(self.run_number(&pipeline.runs[target]))
        };
        (target != view.target).then_some(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builds::BuildStep;

    #[test]
    fn tree_asks_for_relations_and_causes() {
        let tree = tree_query(20);
        assert!(tree.contains("upstreamProjects[fullName]"), "{tree}");
        assert!(
            tree.contains("causes[upstreamProject,upstreamBuild]]]{0,20}"),
            "{tree}"
        );
        assert_eq!(tree.matches("jobs[").count(), FOLDER_DEPTH);
    }

    /// app/build → app/deploy → app/e2e (running), app/deploy also
    /// statically → app/smoke; app/build #11 failed and triggered nothing.
    fn json() -> &'static str {
        r#"{"jobs": [
            {"fullName": "app", "jobs": [
                {"fullName": "app/build", "color": "blue",
                 "downstreamProjects": [{"fullName": "app/deploy"}],
                 "builds": [
                    {"number": 12, "result": "SUCCESS", "timestamp": 1000, "duration": 60000,
                     "actions": [{"causes": [{"shortDescription": "Started by an SCM change"}]}]},
                    {"number": 11, "result": "FAILURE", "timestamp": 500, "duration": 60000},
                    {"number": 10, "result": "SUCCESS", "timestamp": 100, "duration": 60000}
                 ]},
                {"fullName": "app/deploy", "color": "blue",
                 "upstreamProjects": [{"fullName": "app/build"}, {"fullName": "deleted-job"}],
                 "downstreamProjects": [{"fullName": "app/smoke"}],
                 "builds": [
                    {"number": 40, "result": "SUCCESS", "timestamp": 2000, "duration": 1000,
                     "actions": [{"causes": [{"upstreamProject": "app/build", "upstreamBuild": 12}]}]},
                    {"number": 39, "result": "SUCCESS", "timestamp": 300, "duration": 1000,
                     "actions": [{"causes": [{"upstreamProject": "app/build", "upstreamBuild": 10}]}]}
                 ]},
                {"fullName": "app/e2e", "color": "blue_anime", "builds": [
                    {"number": 7, "result": null, "building": true, "timestamp": 3000,
                     "actions": [{}, {"causes": [
                        {"upstreamProject": "app/deploy", "upstreamBuild": 40}]}]}
                ]},
                {"fullName": "app/smoke", "color": "blue", "builds": [
                    {"number": 5, "result": "SUCCESS", "timestamp": 400, "duration": 1000,
                     "actions": [{"causes": [
                        {"upstreamProject": "app/deploy", "upstreamBuild": 39}]}]}
                ]}
            ]},
            {"fullName": "docs", "color": "blue", "builds": []}
        ]}"#
    }

    fn data() -> PipelineData {
        parse(json()).unwrap()
    }

    #[test]
    fn edges_from_configuration_and_causes() {
        let data = data();
        assert_eq!(data.jobs.len(), 5);
        assert_eq!(
            data.edges,
            [
                ("app/build".to_owned(), "app/deploy".to_owned()), // both sides, once
                ("app/deploy".to_owned(), "app/e2e".to_owned()),   // only from a cause
                ("app/deploy".to_owned(), "app/smoke".to_owned()),
            ],
            "unknown jobs dropped"
        );
    }

    #[test]
    fn pipelines_are_named_by_their_common_prefix() {
        let data = data();
        let pipelines = pipelines(&data);
        assert_eq!(
            pipelines.len(),
            1,
            "docs and triggered jobs aren't pipelines"
        );
        let app = &pipelines[0];
        assert_eq!(app.first_job, "app/build");
        assert_eq!(app.name, "app");
        assert_eq!(app.parts.len(), 4);
        let numbers: Vec<u64> = app
            .runs
            .iter()
            .map(|r| data.builds[r.first()].number)
            .collect();
        assert_eq!(numbers, [12, 11, 10], "newest first, failed #11 included");
    }

    #[test]
    fn a_build_triggered_by_an_unloaded_build_starts_no_pipeline() {
        let mut data = data();
        // app/build #12 falls out of the window: deploy #40 still has its
        // cause, so it's no run start and app/deploy no pipeline.
        data.builds
            .retain(|b| !(b.job == "app/build" && b.number == 12));
        let pipelines = pipelines(&data);
        let names: Vec<&str> = pipelines.iter().map(|p| p.first_job.as_str()).collect();
        assert_eq!(names, ["app/build"]);
    }

    #[test]
    fn a_configured_step_started_by_hand_is_no_pipeline() {
        // app/deploy (configured downstream of app/build) started directly,
        // and it triggered app/smoke: still a step of app, not a new pipeline.
        let mut data = data();
        data.builds.push(RunBuild {
            job: "app/deploy".into(),
            number: 41,
            result: Some(JobStatus::Success),
            building: false,
            started: SystemTime::UNIX_EPOCH + Duration::from_millis(5000),
            duration: Duration::ZERO,
            parent: None,
        });
        data.builds.push(RunBuild {
            job: "app/smoke".into(),
            number: 6,
            result: Some(JobStatus::Success),
            building: false,
            started: SystemTime::UNIX_EPOCH + Duration::from_millis(6000),
            duration: Duration::ZERO,
            parent: Some(("app/deploy".into(), 41)),
        });
        let names: Vec<String> = pipelines(&data)
            .iter()
            .map(|p| p.first_job.clone())
            .collect();
        assert_eq!(names, ["app/build"]);
    }

    #[test]
    fn prefixes_stop_at_word_boundaries() {
        let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            common_prefix(&names(&["shop/build", "shop/deploy"])),
            Some("shop".into())
        );
        assert_eq!(
            common_prefix(&names(&["shop-build", "shop-test"])),
            Some("shop".into())
        );
        assert_eq!(
            common_prefix(&names(&["team/a/x", "team/a/y"])),
            Some("team/a".into())
        );
        assert_eq!(
            common_prefix(&names(&["shopping", "shop-x"])),
            None,
            "no half words"
        );
        assert_eq!(
            common_prefix(&names(&["app", "app/deploy"])),
            Some("app".into())
        );
        assert_eq!(common_prefix(&names(&["a/x", "b/y"])), None);
    }

    #[test]
    fn run_status_worst_build_wins_else_partial_until_all_parts_succeed() {
        let data = data();
        let pipeline = &pipelines(&data)[0];
        let status = |i: usize| run_status(&data, pipeline, &pipeline.runs[i]);
        // #12: e2e still running, smoke not reached.
        assert_eq!(status(0), (RunStatus::Partial, true));
        // #11 failed.
        assert_eq!(status(1), (RunStatus::Failure, false));
        // #10 → deploy #39 → smoke #5, but no e2e: not every part succeeded.
        assert_eq!(status(2), (RunStatus::Partial, false));

        let mut all = data.clone();
        all.builds.iter_mut().for_each(|b| {
            b.building = false;
            b.result = Some(JobStatus::Success);
        });
        all.builds.push(RunBuild {
            job: "app/smoke".into(),
            number: 6,
            result: Some(JobStatus::Success),
            building: false,
            started: SystemTime::UNIX_EPOCH,
            duration: Duration::ZERO,
            parent: Some(("app/deploy".into(), 40)),
        });
        let pipeline = &pipelines(&all)[0];
        assert_eq!(
            run_status(&all, pipeline, &pipeline.runs[0]),
            (RunStatus::Success, false)
        );
    }

    #[test]
    fn promotions_are_untriggered_static_downstreams_of_successful_builds() {
        let data = data();
        let pipeline = &pipelines(&data)[0];
        // #12 → deploy #40 → e2e #7 (running): deploy's static downstream
        // app/smoke didn't run; app/build → app/deploy did.
        let labels: Vec<String> = promotions(&data, &pipeline.runs[0])
            .iter()
            .map(Promotion::label)
            .collect();
        assert_eq!(labels, ["app/deploy #40 → app/smoke"]);
        // #11 failed: nothing to promote.
        assert!(promotions(&data, &pipeline.runs[1]).is_empty());
        // #10 → deploy #39 → smoke #5: both steps taken.
        assert!(promotions(&data, &pipeline.runs[2]).is_empty());
    }

    #[test]
    fn only_the_next_promotion_is_offered() {
        // app/build → app/dev (automatic) → app/acc (manual) → app/prod (manual).
        let data = parse(
            r#"{"jobs": [
                {"fullName": "app/build", "color": "blue",
                 "downstreamProjects": [{"fullName": "app/dev"}],
                 "builds": [{"number": 5, "result": "SUCCESS", "timestamp": 1}]},
                {"fullName": "app/dev", "color": "blue",
                 "upstreamProjects": [{"fullName": "app/build"}],
                 "downstreamProjects": [{"fullName": "app/acc"}],
                 "builds": [{"number": 3, "result": "SUCCESS", "timestamp": 2,
                    "actions": [{"causes": [{"upstreamProject": "app/build", "upstreamBuild": 5}]}]}]},
                {"fullName": "app/acc", "color": "blue",
                 "upstreamProjects": [{"fullName": "app/dev"}],
                 "downstreamProjects": [{"fullName": "app/prod"}], "builds": []},
                {"fullName": "app/prod", "color": "blue",
                 "upstreamProjects": [{"fullName": "app/acc"}], "builds": []}
            ]}"#,
        )
        .unwrap();
        let pipeline = &pipelines(&data)[0];
        let labels: Vec<String> = promotions(&data, &pipeline.runs[0])
            .iter()
            .map(Promotion::label)
            .collect();
        assert_eq!(labels, ["app/dev #3 → app/acc"], "not app/prod yet");
    }

    #[test]
    fn missing_parts_explain_partial_runs() {
        let mut data = data();
        let pipeline = pipelines(&data)[0].clone();
        // #10 → deploy #39 → smoke #5: app/e2e never ran in it.
        assert_eq!(
            missing_parts(&data, &pipeline, &pipeline.runs[2]),
            [("app/e2e".to_owned(), false)]
        );
        data.jobs
            .iter_mut()
            .filter(|j| j.name == "app/e2e")
            .for_each(|j| j.status = JobStatus::Disabled);
        assert_eq!(
            missing_parts(&data, &pipeline, &pipeline.runs[2]),
            [("app/e2e".to_owned(), true)],
            "marked when it can't run"
        );
    }

    #[test]
    fn unreached_parts_sit_where_they_would_have_run() {
        let data = data();
        let pipeline = &pipelines(&data)[0];
        let text = |run: &Run| -> Vec<String> {
            run_tree(&data, pipeline, run)
                .iter()
                .map(|row| match &row.item {
                    TreeItem::Build(b) => {
                        let b = &data.builds[run.builds[*b]];
                        format!("{}{} #{}", row.prefix, b.job, b.number)
                    }
                    TreeItem::Missing { job, .. } => format!("{}{job} (not reached)", row.prefix),
                })
                .collect()
        };
        // #12: app/deploy statically triggers app/smoke, which didn't run.
        assert_eq!(
            text(&pipeline.runs[0]),
            [
                "app/build #12",
                "└─ app/deploy #40",
                "   ├─ app/e2e #7",
                "   └─ app/smoke (not reached)",
            ]
        );
        // #11 failed: everything after it is missing, chained.
        assert_eq!(
            text(&pipeline.runs[1]),
            [
                "app/build #11",
                "└─ app/deploy (not reached)",
                "   ├─ app/e2e (not reached)",
                "   └─ app/smoke (not reached)",
            ]
        );
    }

    #[test]
    fn runs_are_trees() {
        let data = data();
        let pipeline = &pipelines(&data)[0];
        let run = &pipeline.runs[0];
        let rows: Vec<String> = run
            .builds
            .iter()
            .zip(&run.prefixes)
            .map(|(&b, prefix)| {
                format!("{prefix}{} #{}", data.builds[b].job, data.builds[b].number)
            })
            .collect();
        assert_eq!(
            rows,
            ["app/build #12", "└─ app/deploy #40", "   └─ app/e2e #7"]
        );
        assert_eq!(run.parents, [None, Some(0), Some(1)]);
    }

    #[test]
    fn runs_list_newest_first_and_filtered() {
        let state = PipelineState::with_data(data());
        assert_eq!(state.visible_runs(), [(0, 0), (0, 1), (0, 2)]);
        let mut filtered = state;
        filtered.runs.filter = "smoke".into();
        assert_eq!(filtered.visible_runs(), [(0, 2)], "a job of the run");
    }

    #[test]
    fn stepping_through_runs() {
        let state = PipelineState::with_data(data());
        let latest = RunView::new(
            "app/build".into(),
            RunRef::Latest,
            crate::app::View::Pipelines,
        );
        assert_eq!(
            state.step_run(&latest, BuildStep::Older),
            Some(RunRef::Number(11))
        );
        assert_eq!(state.step_run(&latest, BuildStep::Newer), None);
        assert_eq!(
            state.step_run(&latest, BuildStep::First),
            Some(RunRef::Number(10))
        );
        let oldest = RunView {
            target: RunRef::Number(11),
            ..latest.clone()
        };
        assert_eq!(
            state.step_run(&oldest, BuildStep::Newer),
            Some(RunRef::Latest)
        );
        assert_eq!(state.find_run(&oldest).map(|(_, i)| i), Some(1));
    }
}
