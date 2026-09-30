//! Talking to a Jenkins instance.

use std::{fmt, time::Duration};

use serde::Deserialize;

use crate::{
    builds::{self, BuildPage, BuildRef},
    config::redact_url,
    console::{self, ConsoleChunk},
    history::{self, HistoryEntry},
    instance::{self, InstanceInfo},
    jobs::{self, Job},
    pipelines::Promotion,
    pipelines::{self, PipelineData},
    promote,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Everything needed to reach one Jenkins instance.
#[derive(Clone, PartialEq, Eq)]
pub struct ConnectionConfig {
    pub url: String,
    /// Extra headers sent with every request (e.g. `Authorization`).
    pub headers: Vec<(String, String)>,
    /// Proxy URL to use, or `None` for a direct connection.
    pub proxy: Option<String>,
    /// Accept invalid TLS certificates. Insecure; off by default.
    pub skip_tls_verify: bool,
}

/// Hand-written so header values and URL/proxy passwords never end up in logs.
impl fmt::Debug for ConnectionConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionConfig")
            .field("url", &redact_url(&self.url))
            .field(
                "headers",
                &self
                    .headers
                    .iter()
                    .map(|(name, _)| name)
                    .collect::<Vec<_>>(),
            )
            .field("proxy", &self.proxy.as_deref().map(redact_url))
            .field("skip_tls_verify", &self.skip_tls_verify)
            .finish()
    }
}

/// What we learned about the instance while connecting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInfo {
    /// From the `X-Jenkins` response header.
    pub version: Option<String>,
    /// The user Jenkins sees us as (`anonymous` without credentials).
    pub user: String,
}

#[derive(Deserialize)]
struct WhoAmI {
    name: String,
}

/// Build an HTTP client for `config`. Env proxy vars are never read here:
/// the proxy decision is made by the caller (see [`crate::proxy`]).
pub fn client(config: &ConnectionConfig) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .user_agent(concat!("leeroy/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .no_proxy()
        // Certs are checked against the OS trust store (rustls-platform-verifier),
        // so internal CAs installed system-wide work without this.
        .danger_accept_invalid_certs(config.skip_tls_verify)
        // For the CSRF crumb, which belongs to a session (see `trigger_build`).
        .cookie_store(true);
    if let Some(proxy) = &config.proxy {
        let proxy = reqwest::Proxy::all(proxy)
            .map_err(|err| format!("invalid proxy {}: {}", redact_url(proxy), chain(&err)))?;
        builder = builder.proxy(proxy);
    }
    builder
        .build()
        .map_err(|err| format!("cannot create HTTP client: {}", chain(&err)))
}

/// Check that the instance is reachable and the credentials work.
///
/// Uses `whoAmI/api/json`, which also works for anonymous users, so a
/// successful result with user `anonymous` means "reachable, not logged in".
pub async fn check(config: &ConnectionConfig) -> Result<ServerInfo, String> {
    let response = get(config, "whoAmI/api/json", false)
        .await?
        .ok_or("not found")?;
    let who: WhoAmI = serde_json::from_str(&response.body)
        .map_err(|_| "unexpected response: not a Jenkins API (is the URL right?)".to_owned())?;
    Ok(ServerInfo {
        version: response.version,
        user: who.name,
    })
}

/// All jobs, folders flattened (see [`crate::jobs::parse_jobs`]).
pub async fn fetch_jobs(config: &ConnectionConfig) -> Result<Vec<Job>, String> {
    let path = format!("api/json?tree={}", jobs::tree_query());
    let response = get(config, &path, false).await?.ok_or("not found")?;
    jobs::parse_jobs(&response.body)
}

/// Build history across all jobs, newest first: every job's newest `limit`
/// builds in one request (see [`crate::history`]).
pub async fn fetch_history(
    config: &ConnectionConfig,
    limit: usize,
) -> Result<Vec<HistoryEntry>, String> {
    let path = format!("api/json?tree={}", history::tree_query(limit));
    let response = get(config, &path, false).await?.ok_or("not found")?;
    history::parse_history(&response.body)
}

/// Job relations and every job's recent builds with their upstream causes
/// (see [`crate::pipelines`]).
pub async fn fetch_pipelines(config: &ConnectionConfig) -> Result<PipelineData, String> {
    let path = format!(
        "api/json?tree={}",
        pipelines::tree_query(pipelines::RUN_BUILDS)
    );
    let response = get(config, &path, false).await?.ok_or("not found")?;
    pipelines::parse(&response.body)
}

/// Details of the instance for the `i` overlay (three small requests).
pub async fn fetch_instance_info(config: &ConnectionConfig) -> Result<InstanceInfo, String> {
    let body = |path: &'static str| async move {
        get(config, path, false)
            .await?
            .map(|r| r.body)
            .ok_or_else(|| format!("{path}: not found"))
    };
    let (root, nodes, queue) = tokio::try_join!(
        body(instance::ROOT_PATH),
        body(instance::NODES_PATH),
        body(instance::QUEUE_PATH)
    )?;
    instance::parse(&root, &nodes, &queue)
}

/// Console output of build `number` of a job, from byte offset `start`.
pub async fn fetch_console(
    config: &ConnectionConfig,
    full_name: &str,
    number: u64,
    start: u64,
) -> Result<ConsoleChunk, String> {
    let path = console::console_path(full_name, number, start);
    let response = get(config, &path, true)
        .await?
        .ok_or_else(|| format!("build #{number} no longer exists"))?;
    let header = |name: &str| {
        response
            .headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    // Without X-Text-Size, assume everything up to what we got.
    let next = header("X-Text-Size")
        .and_then(|v| v.parse().ok())
        .unwrap_or(start + response.bytes.len() as u64);
    Ok(ConsoleChunk {
        more: header("X-More-Data").is_some_and(|v| v.eq_ignore_ascii_case("true")),
        next,
        bytes: response.bytes,
    })
}

/// `which` build of a job. For [`BuildRef::Latest`] this also returns the
/// job's build numbers; a never-built job gives `build: None`. For a number,
/// `build: None` means that build doesn't exist (deleted).
///
/// With `want_numbers`, a numbered build also comes with the job's build
/// numbers (one extra request), needed to step between builds when the view
/// was opened at a specific build (e.g. from the Builds tab).
pub async fn fetch_build(
    config: &ConnectionConfig,
    full_name: &str,
    which: BuildRef,
    want_numbers: bool,
) -> Result<BuildPage, String> {
    let response = get(config, &builds::build_path(full_name, which), true).await?;
    let mut page = match (which, response) {
        (BuildRef::Latest, Some(response)) => {
            let mut page = builds::parse_job_builds(&response.body)?;
            add_stages(config, full_name, &mut page).await;
            return Ok(page);
        }
        (BuildRef::Latest, None) => return Err(format!("job {full_name} no longer exists")),
        (BuildRef::Number(_), Some(response)) => BuildPage {
            build: Some(builds::parse_build(&response.body)?),
            numbers: None,
        },
        (BuildRef::Number(_), None) => BuildPage {
            build: None,
            numbers: None,
        },
    };
    add_stages(config, full_name, &mut page).await;
    if want_numbers {
        let path = builds::numbers_path(full_name);
        if let Some(response) = get(config, &path, true).await? {
            page.numbers = builds::parse_job_builds(&response.body)?.numbers;
        }
    }
    Ok(page)
}

/// Fill in a Pipeline run's stages. Best effort: without the Stage View
/// plugin (404) or on any error the build is shown without them.
async fn add_stages(config: &ConnectionConfig, full_name: &str, page: &mut BuildPage) {
    let Some(build) = page.build.as_mut().filter(|b| b.pipeline) else {
        return;
    };
    let path = builds::stages_path(full_name, build.number);
    build.stages = match get(config, &path, true).await {
        Ok(Some(response)) => match builds::parse_stages(&response.body) {
            Ok(stages) => Some(stages),
            Err(err) => {
                tracing::warn!(%err, "unexpected stage data");
                None
            }
        },
        Ok(None) => None, // no Stage View plugin
        Err(err) => {
            tracing::warn!(%err, "fetching stages failed");
            None
        }
    };
}

struct Response {
    /// Body as text (lossy UTF-8), for JSON.
    body: String,
    /// Raw body, for console output (decoded by the caller across chunks).
    bytes: Vec<u8>,
    headers: reqwest::header::HeaderMap,
    /// From the `X-Jenkins` header.
    version: Option<String>,
}

/// GET `path` (relative to the Jenkins URL) with the configured headers.
/// Non-2xx answers become errors (401/403 explaining who refused and why),
/// except 404 with `allow_not_found`, which gives `Ok(None)`.
async fn get(
    config: &ConnectionConfig,
    path: &str,
    allow_not_found: bool,
) -> Result<Option<Response>, String> {
    let client = client(config)?;
    let get = reqwest::Method::GET;
    send(&client, config, get, path, allow_not_found, &[], None).await
}

/// The CSRF crumb header for POSTs with `client` (none when the protection
/// is off). Jenkins ties it to the session cookie of the request that got it,
/// so use the same client for the POST.
async fn crumb(
    client: &reqwest::Client,
    config: &ConnectionConfig,
) -> Result<Vec<(String, String)>, String> {
    let get = reqwest::Method::GET;
    let response = send(client, config, get, builds::CRUMB_PATH, true, &[], None).await?;
    Ok(match response {
        Some(response) => vec![builds::parse_crumb(&response.body)?],
        None => Vec::new(),
    })
}

/// Whether `job` takes parameters.
async fn is_parameterized(
    client: &reqwest::Client,
    config: &ConnectionConfig,
    job: &str,
) -> Result<bool, String> {
    let path = builds::parameters_path(job);
    let get = reqwest::Method::GET;
    let response = send(client, config, get, &path, true, &[], None)
        .await?
        .ok_or_else(|| format!("job {job} no longer exists"))?;
    builds::has_parameters(&response.body)
}

/// Start a build of `full_name` with its default parameters, like "Build
/// Now".
pub async fn trigger_build(config: &ConnectionConfig, full_name: &str) -> Result<(), String> {
    let client = client(config)?;
    let parameterized = is_parameterized(&client, config, full_name).await?;
    let crumb = crumb(&client, config).await?;
    let path = builds::trigger_path(full_name, parameterized);
    let post = reqwest::Method::POST;
    send(&client, config, post, &path, false, &crumb, None).await?;
    Ok(())
}

/// Take a manual step of a run (see [`crate::promote`]). `Ok(true)`: done by
/// the Build Pipeline plugin, so the build joins the run; `Ok(false)`: no
/// such view, the job was started directly (not linked to the run).
pub async fn promote(config: &ConnectionConfig, promotion: &Promotion) -> Result<bool, String> {
    let client = client(config)?;
    let crumb = crumb(&client, config).await?;
    if let Some((proxy, headers)) = pipeline_proxy(&client, config, &crumb).await? {
        let call = format!("{proxy}/triggerManualBuild");
        let body = (STAPLER_CALL, promote::trigger_arguments(promotion));
        let post = reqwest::Method::POST;
        send(&client, config, post, &call, false, &headers, Some(body)).await?;
        return Ok(true);
    }
    // No Build Pipeline view: start the job with the upstream build's parameters.
    let path = promote::parameters_path(&promotion.from_job, promotion.from_number);
    let get = reqwest::Method::GET;
    let parameters = match send(&client, config, get, &path, true, &[], None).await? {
        Some(response) => promote::parameters(&response.body)?,
        None => Vec::new(),
    };
    start_with(&client, config, &crumb, &promotion.job, &parameters).await?;
    Ok(false)
}

/// What [`rerun`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rerun {
    /// The build that was run again (the latest one's number, when asked
    /// for the latest).
    pub number: u64,
    /// Re-run through the Build Pipeline plugin: the new build joins the
    /// same pipeline run (same upstream cause).
    pub linked: bool,
}

/// Run build `number` of `job` again (`None`: its latest build) with the
/// same parameters, like the Rebuilder plugin. `in_run`: the build is a step
/// of a pipeline run; then the Build Pipeline plugin's own re-run is tried
/// first, which keeps the new build in that run.
pub async fn rerun(
    config: &ConnectionConfig,
    job: &str,
    number: Option<u64>,
    in_run: bool,
) -> Result<Rerun, String> {
    let client = client(config)?;
    let get = reqwest::Method::GET;
    let path = promote::rerun_source_path(job, number);
    let source = send(&client, config, get, &path, true, &[], None)
        .await?
        .ok_or_else(|| match number {
            Some(n) => format!("{job} #{n} no longer exists"),
            None => format!("{job} has never been built"),
        })?;
    let (number, parameters) = promote::rerun_source(&source.body)?;
    let crumb = crumb(&client, config).await?;
    if in_run && let Some((proxy, headers)) = pipeline_proxy(&client, config, &crumb).await? {
        let call = format!("{proxy}/rerunBuild");
        let body = (STAPLER_CALL, promote::rerun_arguments(job, number));
        let post = reqwest::Method::POST;
        // The plugin only re-runs freestyle-type builds; anything else is
        // re-run the generic way below.
        match send(&client, config, post, &call, false, &headers, Some(body)).await {
            Ok(_) => {
                return Ok(Rerun {
                    number,
                    linked: true,
                });
            }
            Err(err) => tracing::debug!(job, number, err, "plugin re-run failed"),
        }
    }
    start_with(&client, config, &crumb, job, &parameters).await?;
    Ok(Rerun {
        number,
        linked: false,
    })
}

/// Content type of a call through a Stapler proxy (what the page's
/// JavaScript sends).
const STAPLER_CALL: &str = "application/x-stapler-method-invocation;charset=UTF-8";

/// The Build Pipeline plugin's proxy of the first Build Pipeline view that
/// has one: `(proxy path, headers for calls through it)`. `None` without
/// such a view.
async fn pipeline_proxy(
    client: &reqwest::Client,
    config: &ConnectionConfig,
    crumb: &[(String, String)],
) -> Result<Option<(String, Vec<(String, String)>)>, String> {
    let get = reqwest::Method::GET;
    let query = promote::views_query();
    let views = match send(client, config, get.clone(), &query, true, &[], None).await? {
        Some(response) => promote::pipeline_views(&response.body)?,
        None => Vec::new(),
    };
    for path in views {
        let Some(page) = send(client, config, get.clone(), &path, true, &[], None).await? else {
            continue;
        };
        // In the page (older Jenkins), or in the script it loads (newer).
        let mut proxy = promote::trigger_proxy(&page.body);
        if proxy.is_none()
            && let Some(script) = promote::proxy_script(&page.body)
            && let Some(js) = send(client, config, get.clone(), &script, true, &[], None).await?
        {
            proxy = promote::trigger_proxy(&js.body);
        }
        let Some((proxy, page_crumb)) = proxy else {
            tracing::debug!(path, "no Build Pipeline trigger found in the view");
            continue;
        };
        let mut headers = crumb.to_vec();
        if !page_crumb.is_empty() {
            // How the page's own calls send it.
            headers.push(("Crumb".to_owned(), page_crumb));
        }
        return Ok(Some((proxy, headers)));
    }
    Ok(None)
}

/// Start `job` with these parameters (form values), or plainly when it
/// takes none.
async fn start_with(
    client: &reqwest::Client,
    config: &ConnectionConfig,
    crumb: &[(String, String)],
    job: &str,
    parameters: &[(String, String)],
) -> Result<(), String> {
    let parameterized = is_parameterized(client, config, job).await?;
    let body = parameterized.then(|| {
        let form = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(parameters)
            .finish();
        ("application/x-www-form-urlencoded", form)
    });
    let path = builds::trigger_path(job, parameterized);
    let post = reqwest::Method::POST;
    send(client, config, post, &path, false, crumb, body).await?;
    Ok(())
}

/// Send a request with the configured headers plus `extra` ones (e.g. the
/// CSRF crumb) and an optional `(content type, body)`. Non-2xx answers
/// become errors (401/403 explaining who refused and why), except 404 with
/// `allow_not_found`: `Ok(None)`.
async fn send(
    client: &reqwest::Client,
    config: &ConnectionConfig,
    method: reqwest::Method,
    path: &str,
    allow_not_found: bool,
    extra: &[(String, String)],
    body: Option<(&str, String)>,
) -> Result<Option<Response>, String> {
    let endpoint = api_url(&config.url, path)?;
    let mut request = client.request(method, endpoint);
    if let Some((content_type, body)) = body {
        request = request
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(body);
    }
    for (name, value) in config.headers.iter().chain(extra) {
        let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| format!("invalid header name {name:?}"))?;
        let mut value = reqwest::header::HeaderValue::from_str(value)
            .map_err(|_| format!("invalid value for header {name}"))?;
        // Keeps the value out of hyper's debug output.
        value.set_sensitive(true);
        request = request.header(name, value);
    }

    let requested = endpoint_origin(&config.url);
    let header_names: Vec<&str> = config.headers.iter().map(|(n, _)| n.as_str()).collect();
    let response = request.send().await.map_err(describe)?;
    let status = response.status();
    let final_url = response.url().clone();
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    let version = header("X-Jenkins");
    // Names only: values may be credentials.
    tracing::debug!(
        path,
        sent_headers = ?header_names,
        %status,
        final_url = %redact_url(final_url.as_str()),
        jenkins = ?version,
        www_authenticate = ?header("WWW-Authenticate"),
        "response"
    );
    if matches!(status.as_u16(), 401 | 403) {
        return Err(auth_failure(
            status.as_u16(),
            version.is_some(),
            header("WWW-Authenticate"),
            requested.as_deref() != Some(&origin(&final_url)),
            &final_url,
        ));
    }
    if allow_not_found && status.as_u16() == 404 {
        return Ok(None);
    }
    if !status.is_success() {
        return Err(format!("unexpected response: HTTP {status}"));
    }
    let headers = response.headers().clone();
    let bytes = response.bytes().await.map_err(describe)?.to_vec();
    Ok(Some(Response {
        body: String::from_utf8_lossy(&bytes).into_owned(),
        bytes,
        headers,
        version,
    }))
}

/// Explain a 401/403: who refused, and whether a redirect dropped our headers.
fn auth_failure(
    status: u16,
    from_jenkins: bool,
    www_authenticate: Option<String>,
    redirected_cross_origin: bool,
    final_url: &url::Url,
) -> String {
    let mut msg = format!("authentication failed (HTTP {status}");
    if from_jenkins {
        msg.push_str(" from Jenkins");
    } else {
        msg.push_str(" from a server in front of Jenkins");
    }
    if let Some(challenge) = www_authenticate {
        msg.push_str(&format!(", asks for: {challenge}"));
    }
    msg.push(')');
    if redirected_cross_origin {
        let mut target = final_url.clone();
        target.set_path("");
        target.set_query(None);
        msg.push_str(&format!(
            ". The request was redirected to {}, which drops the Authorization header \
             for safety: use that as the Jenkins URL",
            redact_url(target.as_str().trim_end_matches('/'))
        ));
    }
    msg
}

/// `scheme://host:port` of a URL, the unit redirects keep credentials within.
fn origin(url: &url::Url) -> String {
    format!(
        "{}://{}:{}",
        url.scheme(),
        url.host_str().unwrap_or_default(),
        url.port_or_known_default().unwrap_or_default()
    )
}

fn endpoint_origin(base: &str) -> Option<String> {
    url::Url::parse(base).ok().map(|u| origin(&u))
}

/// `base` joined with `path`, keeping any path prefix of `base`
/// (e.g. `https://host/jenkins`).
fn api_url(base: &str, path: &str) -> Result<url::Url, String> {
    let mut base = url::Url::parse(base).map_err(|err| format!("invalid Jenkins URL: {err}"))?;
    if !base.path().ends_with('/') {
        base.set_path(&format!("{}/", base.path()));
    }
    base.join(path)
        .map_err(|err| format!("invalid Jenkins URL: {err}"))
}

/// A short, human-readable reason for a failed request.
fn describe(err: reqwest::Error) -> String {
    // The URL is already shown in the UI; drop it from the message.
    let err = err.without_url();
    if err.is_timeout() {
        "timed out".into()
    } else if err.is_connect() {
        let detail = chain(&err);
        match detail.split_once("invalid peer certificate: ") {
            Some((_, reason)) => format!(
                "untrusted TLS certificate ({reason}). Add its CA to the system trust \
                 store, or enable \"Skip TLS verify\" (insecure)"
            ),
            None => format!("cannot connect: {detail}"),
        }
    } else {
        chain(&err)
    }
}

/// An error and its sources, `outer: inner: innermost`, without repeats.
fn chain(err: &dyn std::error::Error) -> String {
    let mut parts = vec![err.to_string()];
    let mut source = err.source();
    while let Some(err) = source {
        let text = err.to_string();
        if !parts.iter().any(|p| p.contains(&text)) {
            parts.push(text);
        }
        source = err.source();
    }
    parts.join(": ")
}

#[cfg(test)]
mod tests {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path},
    };

    use super::*;

    fn config(url: &str) -> ConnectionConfig {
        ConnectionConfig {
            url: url.into(),
            headers: Vec::new(),
            proxy: None,
            skip_tls_verify: false,
        }
    }

    fn whoami(name: &str) -> ResponseTemplate {
        ResponseTemplate::new(200)
            .insert_header("X-Jenkins", "2.504.1")
            .set_body_json(serde_json::json!({ "name": name, "authenticated": true }))
    }

    #[test]
    fn api_url_keeps_path_prefix() {
        assert_eq!(
            api_url("https://host/jenkins", "whoAmI/api/json")
                .unwrap()
                .as_str(),
            "https://host/jenkins/whoAmI/api/json"
        );
        assert_eq!(
            api_url("https://host/", "whoAmI/api/json")
                .unwrap()
                .as_str(),
            "https://host/whoAmI/api/json"
        );
    }

    #[test]
    fn debug_redacts_secrets() {
        let config = ConnectionConfig {
            url: "https://me:urlpass@ci".into(),
            headers: vec![("Authorization".into(), "Basic s3cret".into())],
            proxy: Some("socks5h://u:hunter2@p:1080".into()),
            skip_tls_verify: false,
        };
        let debug = format!("{config:?}");
        for secret in ["s3cret", "hunter2", "urlpass"] {
            assert!(!debug.contains(secret), "{debug}");
        }
        assert!(debug.contains("Authorization"), "{debug}");
    }

    #[tokio::test]
    async fn anonymous_check() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/whoAmI/api/json"))
            .respond_with(whoami("anonymous"))
            .mount(&server)
            .await;

        let info = check(&config(&server.uri())).await.unwrap();
        assert_eq!(
            info,
            ServerInfo {
                version: Some("2.504.1".into()),
                user: "anonymous".into()
            }
        );
    }

    #[tokio::test]
    async fn sends_custom_headers() {
        let server = MockServer::start().await;
        Mock::given(path("/ci/whoAmI/api/json"))
            .and(header("authorization", "Basic bWU6dG9r"))
            .and(header("x-forwarded-user", "me"))
            .respond_with(whoami("me"))
            .mount(&server)
            .await;

        let mut config = config(&format!("{}/ci", server.uri()));
        config.headers = vec![
            ("Authorization".into(), "Basic bWU6dG9r".into()),
            ("X-Forwarded-User".into(), "me".into()),
        ];
        assert_eq!(check(&config).await.unwrap().user, "me");
    }

    #[tokio::test]
    async fn auth_failure() {
        let server = MockServer::start().await;
        Mock::given(path("/whoAmI/api/json"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        assert_eq!(
            check(&config(&server.uri())).await.unwrap_err(),
            "authentication failed (HTTP 401 from a server in front of Jenkins)"
        );
    }

    #[tokio::test]
    async fn auth_failure_says_who_refused() {
        let proxy = MockServer::start().await;
        Mock::given(path("/whoAmI/api/json"))
            .respond_with(
                ResponseTemplate::new(401)
                    .insert_header("WWW-Authenticate", "Basic realm=\"corp\""),
            )
            .mount(&proxy)
            .await;
        let err = check(&config(&proxy.uri())).await.unwrap_err();
        assert!(err.contains("in front of Jenkins"), "{err}");
        assert!(err.contains("realm=\"corp\""), "{err}");

        let jenkins = MockServer::start().await;
        Mock::given(path("/whoAmI/api/json"))
            .respond_with(ResponseTemplate::new(401).insert_header("X-Jenkins", "2.504.1"))
            .mount(&jenkins)
            .await;
        let err = check(&config(&jenkins.uri())).await.unwrap_err();
        assert!(err.contains("from Jenkins"), "{err}");
    }

    /// A redirect to another origin drops Authorization (reqwest does that for
    /// safety); the error must say so instead of a bare 401.
    #[tokio::test]
    async fn cross_origin_redirect_is_explained() {
        let target = MockServer::start().await;
        Mock::given(path("/whoAmI/api/json"))
            .and(header("authorization", "Basic bWU6dG9r"))
            .respond_with(whoami("me"))
            .mount(&target)
            .await;
        Mock::given(path("/whoAmI/api/json"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&target)
            .await;
        let redirector = MockServer::start().await;
        Mock::given(path("/whoAmI/api/json"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("Location", format!("{}/whoAmI/api/json", target.uri())),
            )
            .mount(&redirector)
            .await;

        let mut config = config(&redirector.uri());
        config.headers = vec![("Authorization".into(), "Basic bWU6dG9r".into())];
        let err = check(&config).await.unwrap_err();
        assert!(err.contains("redirected to"), "{err}");
        assert!(err.contains(&target.uri()), "{err}");

        // Using the target directly works.
        config.url = target.uri();
        assert_eq!(check(&config).await.unwrap().user, "me");
    }

    #[tokio::test]
    async fn fetches_jobs_with_headers_and_tree() {
        let server = MockServer::start().await;
        Mock::given(path("/ci/api/json"))
            .and(wiremock::matchers::query_param("tree", jobs::tree_query()))
            .and(header("x-forwarded-user", "me"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "jobs": [
                    {"name": "b", "fullName": "b", "url": "u/b", "color": "red"},
                    {"name": "f", "fullName": "f", "url": "u/f", "jobs": [
                        {"name": "a", "fullName": "f/a", "url": "u/a", "color": "blue_anime"}
                    ]}
                ]
            })))
            .mount(&server)
            .await;

        let mut config = config(&format!("{}/ci", server.uri()));
        config.headers = vec![("X-Forwarded-User".into(), "me".into())];
        let jobs = fetch_jobs(&config).await.unwrap();
        let names: Vec<&str> = jobs.iter().map(|j| j.full_name.as_str()).collect();
        assert_eq!(names, ["b", "f/a"]);
        assert!(jobs[1].building);
    }

    #[tokio::test]
    async fn fetches_builds_latest_and_numbered() {
        let server = MockServer::start().await;
        Mock::given(path("/job/team/job/my%20svc/api/json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "allBuilds": [{"number": 3}, {"number": 1}],
                "lastBuild": {"number": 3, "result": "SUCCESS", "building": false,
                              "timestamp": 1_700_000_000_000u64, "duration": 1000}
            })))
            .mount(&server)
            .await;
        Mock::given(path("/job/team/job/my%20svc/1/api/json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "number": 1, "result": "FAILURE", "timestamp": 0, "duration": 0
            })))
            .mount(&server)
            .await;
        Mock::given(path("/job/never/api/json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"allBuilds": [], "lastBuild": null})),
            )
            .mount(&server)
            .await;

        let config = config(&server.uri());
        let latest = fetch_build(&config, "team/my svc", BuildRef::Latest, false)
            .await
            .unwrap();
        assert_eq!(latest.numbers, Some(vec![1, 3]));
        assert_eq!(latest.build.unwrap().number, 3);
        let first = fetch_build(&config, "team/my svc", BuildRef::Number(1), false)
            .await
            .unwrap();
        assert_eq!(first.build.unwrap().number, 1);
        assert_eq!(first.numbers, None);
        // Deleted build: not found, not an error.
        let gone = fetch_build(&config, "team/my svc", BuildRef::Number(2), false)
            .await
            .unwrap();
        assert_eq!(gone.build, None);
        let never = fetch_build(&config, "never", BuildRef::Latest, false)
            .await
            .unwrap();
        assert_eq!(
            never,
            BuildPage {
                build: None,
                numbers: Some(vec![])
            }
        );
        // A job that's gone is an error.
        assert!(
            fetch_build(&config, "gone", BuildRef::Latest, false)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn fetches_console_chunks() {
        let server = MockServer::start().await;
        Mock::given(path("/job/team/job/svc/7/logText/progressiveText"))
            .and(wiremock::matchers::query_param("start", "10"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("X-Text-Size", "25")
                    .insert_header("X-More-Data", "true")
                    .set_body_bytes(&b"more output\n\xc3"[..]),
            )
            .mount(&server)
            .await;
        let chunk = fetch_console(&config(&server.uri()), "team/svc", 7, 10)
            .await
            .unwrap();
        assert_eq!(chunk.next, 25);
        assert!(chunk.more);
        assert_eq!(
            chunk.bytes, b"more output\n\xc3",
            "raw bytes, not lossy text"
        );

        // Finished build: no X-More-Data.
        Mock::given(path("/job/done/1/logText/progressiveText"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("X-Text-Size", "3")
                    .set_body_string("ok\n"),
            )
            .mount(&server)
            .await;
        let chunk = fetch_console(&config(&server.uri()), "done", 1, 0)
            .await
            .unwrap();
        assert!(!chunk.more);
        assert!(
            fetch_console(&config(&server.uri()), "gone", 1, 0)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn fetches_history_in_one_request() {
        let server = MockServer::start().await;
        Mock::given(path("/api/json"))
            .and(wiremock::matchers::query_param("tree", history::tree_query(3)))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "jobs": [
                    {"fullName": "a", "builds": [
                        {"number": 2, "result": "SUCCESS", "timestamp": 2000, "duration": 1},
                        {"number": 1, "result": "FAILURE", "timestamp": 500, "duration": 1}
                    ]},
                    {"fullName": "b", "builds": [
                        {"number": 7, "result": null, "building": true, "timestamp": 1000, "duration": 0}
                    ]}
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;
        let entries = fetch_history(&config(&server.uri()), 3).await.unwrap();
        let ids: Vec<(&str, u64)> = entries.iter().map(|e| (e.job.as_str(), e.number)).collect();
        assert_eq!(ids, [("a", 2), ("b", 7), ("a", 1)]);
    }

    #[tokio::test]
    async fn numbered_build_with_numbers() {
        let server = MockServer::start().await;
        Mock::given(path("/job/a/4/api/json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "number": 4, "result": "SUCCESS", "timestamp": 0, "duration": 0
            })))
            .mount(&server)
            .await;
        Mock::given(path("/job/a/api/json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "allBuilds": [{"number": 5}, {"number": 4}, {"number": 2}]
            })))
            .mount(&server)
            .await;
        let page = fetch_build(&config(&server.uri()), "a", BuildRef::Number(4), true)
            .await
            .unwrap();
        assert_eq!(page.build.unwrap().number, 4);
        assert_eq!(page.numbers, Some(vec![2, 4, 5]));
    }

    #[tokio::test]
    async fn not_jenkins() {
        let server = MockServer::start().await;
        Mock::given(path("/whoAmI/api/json"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>"))
            .mount(&server)
            .await;
        let err = check(&config(&server.uri())).await.unwrap_err();
        assert!(err.contains("not a Jenkins API"), "{err}");
    }

    #[tokio::test]
    async fn connection_refused() {
        // Bind and drop to get a port nothing listens on.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let err = check(&config(&format!("http://127.0.0.1:{port}")))
            .await
            .unwrap_err();
        assert!(err.starts_with("cannot connect"), "{err}");
    }

    /// The mock server acts as an HTTP proxy for a host that doesn't resolve:
    /// success proves the proxy is used and DNS happens on the proxy side.
    #[tokio::test]
    async fn goes_through_http_proxy() {
        let proxy = MockServer::start().await;
        Mock::given(path("/whoAmI/api/json"))
            .and(header("host", "jenkins.invalid"))
            .respond_with(whoami("anonymous"))
            .mount(&proxy)
            .await;

        let mut config = config("http://jenkins.invalid");
        config.proxy = Some(proxy.uri());
        assert_eq!(check(&config).await.unwrap().user, "anonymous");
    }
}
