//! Details of the connected Jenkins instance, for the `i` overlay: model,
//! request paths, parsing. No IO.
//!
//! Three small requests: the root (`mode`, `useSecurity`, `quietingDown`),
//! the nodes with their executors (`computer`), and the build queue.

use serde::Deserialize;

/// Request paths, relative to the Jenkins URL.
pub const ROOT_PATH: &str = "api/json?tree=mode,nodeDescription,useSecurity,quietingDown";
pub const NODES_PATH: &str =
    "computer/api/json?tree=busyExecutors,totalExecutors,computer[displayName,offline]";
pub const QUEUE_PATH: &str = "queue/api/json?tree=items[id]";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceInfo {
    /// `NORMAL` (anyone may use the controller's executors) or `EXCLUSIVE`.
    pub mode: Option<String>,
    /// The controller's description, if set.
    pub description: Option<String>,
    pub security: bool,
    /// Preparing for shutdown: no new builds start.
    pub quieting_down: bool,
    pub nodes_online: usize,
    pub nodes_total: usize,
    pub busy_executors: usize,
    pub total_executors: usize,
    /// Builds waiting in the queue.
    pub queued: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRoot {
    mode: Option<String>,
    node_description: Option<String>,
    #[serde(default)]
    use_security: bool,
    #[serde(default)]
    quieting_down: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawNodes {
    #[serde(default)]
    busy_executors: usize,
    #[serde(default)]
    total_executors: usize,
    #[serde(default)]
    computer: Vec<RawNode>,
}

#[derive(Deserialize)]
struct RawNode {
    #[serde(default)]
    offline: bool,
}

#[derive(Deserialize)]
struct RawQueue {
    #[serde(default)]
    items: Vec<serde_json::Value>,
}

fn json<'a, T: Deserialize<'a>>(what: &str, text: &'a str) -> Result<T, String> {
    serde_json::from_str(text).map_err(|err| format!("unexpected {what} data: {err}"))
}

/// Combine the three answers.
pub fn parse(root: &str, nodes: &str, queue: &str) -> Result<InstanceInfo, String> {
    let root: RawRoot = json("instance", root)?;
    let nodes: RawNodes = json("node", nodes)?;
    let queue: RawQueue = json("queue", queue)?;
    Ok(InstanceInfo {
        mode: root.mode,
        description: root.node_description.filter(|d| !d.trim().is_empty()),
        security: root.use_security,
        quieting_down: root.quieting_down,
        nodes_online: nodes.computer.iter().filter(|c| !c.offline).count(),
        nodes_total: nodes.computer.len(),
        busy_executors: nodes.busy_executors,
        total_executors: nodes.total_executors,
        queued: queue.items.len(),
    })
}

/// Loading state of the overlay.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum InfoLoad {
    #[default]
    NotLoaded,
    Loading,
    Loaded(InstanceInfo),
    Failed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_three_answers() {
        let info = parse(
            r#"{"_class": "hudson.model.Hudson", "mode": "NORMAL",
                "nodeDescription": "the controller", "useSecurity": true,
                "quietingDown": false}"#,
            r#"{"busyExecutors": 1, "totalExecutors": 4, "computer": [
                {"displayName": "Built-In Node", "offline": false},
                {"displayName": "agent-1", "offline": false},
                {"displayName": "agent-2", "offline": true}]}"#,
            r#"{"items": [{"id": 7}, {"id": 8}]}"#,
        )
        .unwrap();
        assert_eq!(
            info,
            InstanceInfo {
                mode: Some("NORMAL".into()),
                description: Some("the controller".into()),
                security: true,
                quieting_down: false,
                nodes_online: 2,
                nodes_total: 3,
                busy_executors: 1,
                total_executors: 4,
                queued: 2,
            }
        );
    }

    #[test]
    fn missing_fields_default_and_bad_json_fails() {
        let info = parse(r#"{"nodeDescription": " "}"#, "{}", "{}").unwrap();
        assert_eq!(info.description, None, "blank description");
        assert_eq!((info.nodes_total, info.queued), (0, 0));
        assert!(parse("<html>", "{}", "{}").is_err());
    }
}
