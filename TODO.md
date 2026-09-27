# TODO

- [ ] **Caching elsewhere**: investigate other views that could benefit from
  caching to reduce UI sluggishness. First suspects, from how they work now:
  - Jobs tab: `JobsState::visible()` filters (and lowercases) every job on each
    call, several calls per keypress and frame.
  - Builds tab: `HistoryState::visible()` / `has_more()` format and lowercase
    `job #number` for every loaded build on each call.
  - Run view: `run_tree` / `missing_parts` scan all edges and jobs per row,
    every frame.
  Measure first (like `pipeline_runs_speed`) with a big instance.
