## Jobs

All jobs of the instance, with folders and multibranch projects flattened into
`folder/sub/job` names, their last result (colour *and* word) and `⟳` while a
build runs.

| Key | Action |
| --- | --- |
| `↑/↓` `j/k`, `PgUp/PgDn`, `g/G` | navigation |
| `Enter` | details of the job's most recent build |
| `b` | re-run the job's latest build with the same parameters (asks first) |
| `B` | start a new build of the job (asks first; default parameter values) |
| `/` | filter (live, case-insensitive; space-separated words must all match) |
| `Enter` / `Esc` while filtering | apply / cancel |
| `Esc` | clear an applied filter |

The header's right end shows how long ago the data was fetched: `⟳ 4s`. The
icon is green while auto-refresh is on and gray while it's off; `⟳ …` means a
refresh is running and `✕` that the last one failed. The
default and the interval are in the settings (`[refresh]`); a failed refresh is
retried after a full interval, never in a tight loop.

## Last build

`Enter` on a job shows its most recent build: result, when it started, how long
it took, what triggered it, parameters, description and the SCM changes it
included. A running build shows its elapsed time against Jenkins' estimate as a
progress bar. For a Pipeline (Jenkinsfile) build, its stages are shown in order,
`● Checkout 4s ─▶ ● Build 1m 02s ─▶ ✕ Test 38s ─▶ ○ Deploy`, when Jenkins has the
Pipeline Stage View plugin (its `wfapi`); without it the section is left out
([no stages](Troubleshooting.md#a-pipeline-build-has-no-stages)). `r` and
auto-refresh re-fetch the build while this view is open (so a running build
updates live), and the header's `⟳` age refers to it.

| Key | Action |
| --- | --- |
| `←` / `→` | older / newer build (deleted builds are skipped) |
| `Home` / `End` | first (oldest kept) / latest build |
| `c` | console output of this build |
| `b` | re-run this build with the same parameters (asks first) |
| `B` | start a new build of this job (asks first; default parameter values) |
| `↑/↓` `j/k`, `PgUp/PgDn`, `g/G` | navigation |
| `Esc` | back to the list |

The title shows which build you're on (`#42 · 17 of 20`, counted from the
oldest). On the latest build, refreshing follows new builds as they start; an
older build stays put.

## Console output

`c` in the build view opens the build's console output, scrolled to the end.
While the build runs, new output is fetched at the refresh interval while
auto-refresh is on (only the new part) and followed at the bottom. Scrolling up pauses following (the title says so);
`End` resumes it.

| Key | Action |
| --- | --- |
| `↑/↓` `j/k`, `PgUp/PgDn` | navigation |
| `Home` / `End` | top / bottom (`End` = follow again) |
| `←/→` | scroll long lines sideways |
| `c` / `Esc` | back to the build (`c` toggles the console) |
| `b` | re-run this build with the same parameters (asks first) |
| `B` | start a new build of this job (asks first; default parameter values) |

Colour codes are removed, `\r` progress bars show their final state, and at
most the last 100 000 lines are kept.

## Builds

The build history of all jobs, newest first: status, job and build number, when
it started and how long it took (`…` while running). `Enter` opens the build (with
`←/→`, `c` etc. as usual); `Esc` comes back here. `b` re-runs the selected
build with the same parameters, `B` starts a new build of its job (both ask
first). `/` filters like on the Jobs tab.

Jenkins has no "newest builds overall" API, so Leeroy asks every job for its
newest *N* builds in **one** request (`tree=jobs[…,builds[…]{0,N}]`), where *N* is
the number of rows that fit on your screen, and merges them. That's exact for the
*N* rows shown. Moving down to within a screenful of the last row loads the next
three screenfuls in the background (again one request), so paging down rarely
waits; if you get to the end first, the selection moves on once they arrive.
Refreshes only reload down to just below the selection, so after scrolling back
up they're one screenful again. Filtering by job name or `#number` is complete without extra requests;
filtering by result (e.g. "failed") only searches what's loaded.
