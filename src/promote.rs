//! Promotions: taking a run's manual steps (see [`crate::pipelines::promotions`]).
//! Request paths and parsing. No IO.
//!
//! The Build Pipeline plugin, whose "Manually Execute Downstream Project"
//! steps these usually are, has no REST endpoint for them: its trigger is a
//! JavaScript method of a Build Pipeline *view*, reached through a proxy URL
//! that the view's page embeds (bound to the HTTP session). Called like that,
//! the plugin records an upstream cause (the build joins the run) and passes
//! the upstream build's parameters on. So:
//!
//! 1. find a Build Pipeline view (`api/json?tree=views[...]`);
//! 2. load its page and take the proxy URL from `makeStaplerProxy(...)`;
//! 3. POST `[upstream number, job, upstream job]` to `<proxy>/triggerManualBuild`.
//!
//! Without such a view, the job is started directly with the upstream build's
//! parameters: it runs, but without an upstream cause it isn't linked to the run.

use serde::Deserialize;

use crate::pipelines::Promotion;

pub const PIPELINE_VIEW_CLASS: &str =
    "au.com.centrumsystems.hudson.plugin.buildpipeline.BuildPipelineView";

/// `tree` query for the views of the instance and of its folders.
pub fn views_query() -> String {
    let mut tree = String::from("jobs[name,views[name,_class]]");
    for _ in 1..crate::jobs::FOLDER_DEPTH {
        tree = format!("jobs[name,views[name,_class],{tree}]");
    }
    format!("api/json?tree=views[name,_class],{tree}")
}

#[derive(Deserialize)]
struct RawNode {
    #[serde(default)]
    name: String,
    #[serde(default)]
    views: Vec<RawView>,
    #[serde(default)]
    jobs: Vec<RawNode>,
}

#[derive(Deserialize)]
struct RawView {
    name: String,
    #[serde(rename = "_class", default)]
    class: String,
}

/// Page paths of the Build Pipeline views, top-level ones first, then
/// those in folders (`job/team/view/Delivery/`).
pub fn pipeline_views(json: &str) -> Result<Vec<String>, String> {
    let root: RawNode =
        serde_json::from_str(json).map_err(|err| format!("unexpected view data: {err}"))?;
    let mut paths = Vec::new();
    fn walk(node: &RawNode, folder: &str, paths: &mut Vec<String>) {
        let prefix = if folder.is_empty() {
            String::new()
        } else {
            crate::builds::job_path(folder)
        };
        for view in node.views.iter().filter(|v| v.class == PIPELINE_VIEW_CLASS) {
            paths.push(format!("{prefix}{}", view_path(&view.name)));
        }
        for child in &node.jobs {
            let name = if folder.is_empty() {
                child.name.clone()
            } else {
                format!("{folder}/{}", child.name)
            };
            walk(child, &name, paths);
        }
    }
    walk(&root, "", &mut paths);
    Ok(paths)
}

/// Path of a view's page (relative to its folder).
pub fn view_path(name: &str) -> String {
    let encoded: String = name
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("view/{encoded}/")
}

/// The proxy of the plugin's trigger: `(path, crumb)`, from
/// `makeStaplerProxy('<path>','<crumb>',[...,'triggerManualBuild',...])`.
/// Older Jenkins versions write that call into the view page itself; newer
/// ones load it from a script (see [`proxy_script`]). The path is absolute on
/// the server (`/jenkins/$stapler/bound/…`).
pub fn trigger_proxy(text: &str) -> Option<(String, String)> {
    let mut rest = text;
    while let Some(start) = rest.find("makeStaplerProxy(") {
        rest = &rest[start + "makeStaplerProxy(".len()..];
        let end = rest.find(')')?;
        let call = &rest[..end];
        if call.contains("'triggerManualBuild'") || call.contains("\"triggerManualBuild\"") {
            let mut args = call.split(',').map(|a| a.trim().trim_matches(['\'', '"']));
            let path = args.next()?.to_owned();
            let crumb = args.next().unwrap_or_default().to_owned();
            return path.contains("/$stapler/bound/").then_some((path, crumb));
        }
    }
    None
}

/// Newer Jenkins versions (Content Security Policy) don't write the proxy
/// into the page: they load it with `<script src="…/$stapler/bound/script/…
/// ?var=…&methods=…,triggerManualBuild,…">`, whose answer is the
/// `makeStaplerProxy(...)` call. This finds that script's URL.
pub fn proxy_script(html: &str) -> Option<String> {
    html.split("src=")
        .skip(1)
        .filter_map(|rest| {
            let quote = rest.chars().next().filter(|q| *q == '"' || *q == '\'')?;
            let value = &rest[1..];
            Some(value[..value.find(quote)?].replace("&amp;", "&"))
        })
        .find(|src| src.contains("$stapler/bound/script") && src.contains("triggerManualBuild"))
}

/// Body of the `triggerManualBuild` call: its arguments as a JSON array. Job
/// names are absolute (leading `/`), so they're found whether the view is at
/// the top level or in a folder (the plugin looks names up from the view's
/// folder).
pub fn trigger_arguments(promotion: &Promotion) -> String {
    serde_json::json!([
        promotion.from_number,
        format!("/{}", promotion.job),
        format!("/{}", promotion.from_job)
    ])
    .to_string()
}

/// Request path for a build's parameters.
pub fn parameters_path(job: &str, number: u64) -> String {
    format!(
        "{}{number}/api/json?tree=actions[parameters[name,value]]",
        crate::builds::job_path(job)
    )
}

#[derive(Deserialize)]
struct RawActions {
    #[serde(default)]
    actions: Vec<Option<RawParameters>>,
}

#[derive(Deserialize)]
struct RawParameters {
    #[serde(default)]
    parameters: Vec<RawParameter>,
}

#[derive(Deserialize)]
struct RawParameter {
    name: String,
    value: Option<serde_json::Value>,
}

/// A build's parameters as form values (passwords, which have no value in
/// the API, are left out: the downstream job uses its default).
pub fn parameters(json: &str) -> Result<Vec<(String, String)>, String> {
    let raw: RawActions =
        serde_json::from_str(json).map_err(|err| format!("unexpected build data: {err}"))?;
    Ok(raw
        .actions
        .into_iter()
        .flatten()
        .flat_map(|a| a.parameters)
        .filter_map(|p| {
            let value = match p.value? {
                serde_json::Value::String(s) => s,
                serde_json::Value::Null => return None,
                other => other.to_string(),
            };
            Some((p.name, value))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn promotion() -> Promotion {
        Promotion {
            from_job: "shop/test".into(),
            from_number: 49,
            job: "shop/deploy".into(),
        }
    }

    #[test]
    fn finds_build_pipeline_views_also_in_folders() {
        assert!(views_query().contains("jobs[name,views[name,_class],jobs["));
        let views = pipeline_views(
            r#"{"views": [
                {"_class": "hudson.model.AllView", "name": "all"},
                {"_class": "au.com.centrumsystems.hudson.plugin.buildpipeline.BuildPipelineView",
                 "name": "Shop delivery"}],
              "jobs": [{"name": "puppetry", "views": [], "jobs": [
                {"name": "hyp", "views": [
                    {"_class": "au.com.centrumsystems.hudson.plugin.buildpipeline.BuildPipelineView",
                     "name": "hypprod"}]}]}]}"#,
        )
        .unwrap();
        assert_eq!(
            views,
            [
                "view/Shop%20delivery/",
                "job/puppetry/job/hyp/view/hypprod/"
            ]
        );
    }

    #[test]
    fn takes_the_trigger_proxy_from_the_page() {
        let html = r#"<script>var other = makeStaplerProxy('/jenkins/$stapler/bound/aaa','c1',['getBuild']);
            var buildPipelineView = makeStaplerProxy('/jenkins/$stapler/bound/0f3e-42','c2',['getProjectBuildPipelineTree','triggerManualBuild','rerunBuild']);</script>"#;
        assert_eq!(
            trigger_proxy(html),
            Some(("/jenkins/$stapler/bound/0f3e-42".into(), "c2".into()))
        );
        assert_eq!(trigger_proxy("<html>no proxy</html>"), None);
    }

    #[test]
    fn newer_jenkins_loads_the_proxy_from_a_script() {
        let html = r#"<script src="/jenkins/static/abc/scripts/behavior.js"></script>
            <script src="/jenkins/$stapler/bound/script/jenkins/$stapler/bound/0f3e-42?var=buildPipelineView&amp;methods=getProjectBuildPipelineTree,triggerManualBuild" type="text/javascript"></script>"#;
        assert_eq!(trigger_proxy(html), None, "not in the page itself");
        assert_eq!(
            proxy_script(html).as_deref(),
            Some(
                "/jenkins/$stapler/bound/script/jenkins/$stapler/bound/0f3e-42?var=buildPipelineView&methods=getProjectBuildPipelineTree,triggerManualBuild"
            )
        );
        // The script's answer is the call.
        let script = "buildPipelineView = makeStaplerProxy('/jenkins/$stapler/bound/0f3e-42','crumb-9',['getProjectBuildPipelineTree','triggerManualBuild']);";
        assert_eq!(
            trigger_proxy(script),
            Some(("/jenkins/$stapler/bound/0f3e-42".into(), "crumb-9".into()))
        );
    }

    #[test]
    fn call_arguments_and_parameters() {
        assert_eq!(
            trigger_arguments(&promotion()),
            r#"[49,"/shop/deploy","/shop/test"]"#
        );
        assert!(parameters_path("shop/test", 49).starts_with("job/shop/job/test/49/api/json"));
        let params = parameters(
            r#"{"actions": [{}, null, {"parameters": [
                {"name": "ENV", "value": "staging"}, {"name": "DRY_RUN", "value": false},
                {"name": "TOKEN"}]}]}"#,
        )
        .unwrap();
        assert_eq!(
            params,
            [
                ("ENV".to_owned(), "staging".to_owned()),
                ("DRY_RUN".to_owned(), "false".to_owned())
            ]
        );
    }
}
