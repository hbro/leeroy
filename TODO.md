# TODO

- [x] **Theming support**
  - Convert the existing colors to variables that a theme defines.
  - Create a dark and a light theme based on the current colors.
  - Make the theme selectable in the settings.
  - Redraw the UI live when switching themes.

- [x] **Semantic versioning** of the application.

- [x] **GitHub Actions build pipeline** that builds automatically for several
  platforms, starting with:
  - Linux x86_64
  - Linux ARM
  - macOS
  - Windows x86_64

- [x] **Pipelines tab** that draws diagrams of how jobs relate (upstream/downstream etc.).
  - Probably also needs a **pipeline runs** tab.

- [x] **Remove the `s` shortcut for settings**: the help popup still lists `s`
  for opening the settings, but that's `0` now; `s` shouldn't do it anymore.

- [x] **Promotions from a pipeline run**: in the run view, `p` opens an overlay
  listing the run's available (manual) promotion targets. Space toggles a target
  (`[x]`, several at once), Enter confirms and promotes.

- [x] **Start a pipeline run**: on the Pipelines tab (selected pipeline) and in
  the run view, trigger a fresh run of that pipeline.

- [x] **Filters remembered per tab**: each tab keeps its own applied filter when
  switching tabs (already the case; guarded by `each_tab_remembers_its_filter`).

- [x] **Instance info overlay**: hotkey `i` shows an overlay with information
  about the connected Jenkins instance.

- [x] **Help section naming**: the help popup's section for the current view is
  titled after the view (e.g. "Pipelines"); label it "Navigation" instead.

- [x] **Open in the browser**: a hotkey that opens the item on screen (job,
  build, console, pipeline run) in Jenkins' web UI.

- [x] **Start a build of a job**: `b` on the Jobs tab starts a build of the
  selected job; in the build view (and console), `b` starts a new build of the
  job the shown build belongs to. Same y/n prompt as for pipelines.
