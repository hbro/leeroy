A pipeline is a job whose own builds (not triggered by another job) trigger
other jobs, e.g. an SCM-triggered `app/build` that starts `app/deploy`, which
starts `app/e2e`. A job the configuration says another job triggers (a
downstream or manual step) stays a step of that pipeline, even when it's started
by hand. The tab lists every pipeline with the status of its latest
run, when that started and how long it took (`…` while running). It's named
after the common prefix of its jobs (`app` for `app/build`, `app/deploy`, …, cut
at `/`, `-`, `_` or `.`), or after its first job when there's none or two
pipelines would share it.

A run's status: **failure**, **unstable** or **aborted** if any of its builds
ended that way (the worst wins); otherwise **success** when every part of the
pipeline (every job it can trigger) has a successful build in the run, and
**partial** while some part hasn't (still running, or not reached). `⟳` marks a
run that's still going.

| Key | Action |
| --- | --- |
| `↑/↓` `j/k`, `PgUp/PgDn`, `g/G` | navigation |
| `Enter` | open the latest run |
| `B` | start a new run (asks first; default parameter values) |
| `/` | filter by name |

Leeroy finds the relations in two places, because Jenkins records them in two
incomplete ways: the job configuration (freestyle "build other projects" and
`upstream` triggers) and the *upstream cause* of every triggered build (which
also covers a Jenkinsfile's `build job:` step). One request asks every job for
both, looking at its last 20 builds, so runs older than that aren't shown.

## Pipeline runs

Every run of every pipeline, newest first, named like its pipeline plus the
number of the build that started it (`app #12`). `Enter` opens it in the run
view; `B` starts a new run of its pipeline (asks first); `/` filters by pipeline
name or any `job #number` in the run.

## Run view

One run as a tree: every build under the build that triggered it, with when it
started and how long it took. `v` switches to boxes, stacked the same way (each
indented under its trigger, with arrows into it, borders in the build's status
colour) and back. When the boxes don't fit, the view scrolls to keep the
selected one in sight and the title says where there's more (`more ↓`). Parts
of the pipeline the run never reached are shown dimmed where they would have
run, under the job that triggers them (`○ deploy/production  not reached`, or
`disabled`): that's why a run whose builds all succeeded can still be partial.

| Key | Action |
| --- | --- |
| `←` / `→` | older / newer run of the pipeline |
| `Home` / `End` | first (oldest loaded) / latest run |
| `v` | tree ⇄ boxes (view) |
| `↑/↓` `j/k` | navigation |
| `Enter` | open the selected build (details, console with `c`) |
| `p` | promote: the run's manual steps, `Space` selects several, `Enter` takes them |
| `b` | re-run the selected build (step) with the same parameters, in the same run (asks first) |
| `B` | start a new run of the pipeline (asks first; default parameter values) |
| `Esc` | back to the list |

## Promotions

`p` in the run view lists the run's manual steps: every job downstream (in the
job configuration) of a successfully finished build of the run that it hasn't
triggered, e.g. `shop/test #49 → shop/deploy` for a Build Pipeline plugin
"Manually Execute Downstream Project" step. `Space` selects one or more, `Enter`
takes the selected ones; nothing happens until at least one is selected. Only steps
that can be taken now are offered (after dev → acc → prod's dev, only acc), and a
step just taken isn't offered again until its build shows up.

Leeroy takes a step the way the Build Pipeline plugin's own page does, through a
Build Pipeline view (at the top level or in a folder): the new build then has
the upstream build as its cause
(it joins the run) and gets its parameters. Without such a view, the job is
started directly with the upstream build's parameters; it runs, but isn't linked
to the run (the notice says so; see
[Troubleshooting](Troubleshooting.md#a-promotion-or-re-run-isnt-part-of-the-run)).

## Re-running

`b` re-runs a build: its job is started again with the same parameter values
(like the Rebuilder plugin, but needing no plugin). A step re-run from the run
view (or from a build opened there) goes through the Build Pipeline plugin's own
re-run when a Build Pipeline view exists, so the new build keeps its upstream
cause and stays in the same run; otherwise it's re-run the generic way and isn't
linked to the run (the notice says so). `B` always starts a new build or
pipeline run with the default parameter values.
