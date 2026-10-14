Content lives in tabs, selected with the digit keys shown in the tab bar:

| Key | Tab |
| --- | --- |
| `1` | Jobs |
| `2` | Builds (build history) |
| `3` | Pipelines |
| `4` | Pipeline runs (of all pipelines) |
| `0` | Settings (far right of the tab bar) |

Global keys, available everywhere except while typing in a field (the full list
is in the help popup, `h` or `?`, hinted at the right of the header):

| Key | Action |
| --- | --- |
| `q` | quit (asks first: `y`/`Enter`/`q` quits, `n`/`Esc` stays; setting `ui.confirm_quit`) |
| `Ctrl-C` | quit immediately, from anywhere |
| `h` / `?` | help (with Leeroy's version and build time at the bottom) |
| `o` | open what's on screen (job, build, console, pipeline, run) in Jenkins' web UI |
| `i` | about the Jenkins instance: version, user, nodes, executors, queue, and how Leeroy connects (TLS, proxy, header names) |
| `r` | refresh now (reconnects if the connection failed) |
| `R` | toggle auto-refresh for this session |
| `t` | when builds started: relative (`17m ago`) ⇄ absolute (`2026-09-29 14:03`, local time). Remembered: saved as `ui.timestamps` (only for the session while `LEEROY_UI_TIMESTAMPS` is set) |
