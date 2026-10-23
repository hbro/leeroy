# TODO

- [ ] **Parameters when starting**: when starting a new build or pipeline run
  (`B`), let the user choose between the default parameters and entering
  them (a form with the job's parameter definitions, defaults filled in).
- [ ] **Sorting**: a global key (e.g. `s`, currently free) to sort the list
  views: by status of the latest build/run, name (the default), time of the
  latest build/run, or its duration; each also reversed. (`s` used to open
  the settings; test `settings_only_via_their_tab_key` checks it does nothing
  now, so it changes along.)
- [ ] **Auth cleanups** (skipped in the `/simplify` of the login modes):
  - Structured OAuth errors: `oidc.rs` returns the provider's `error` code
    (`invalid_client`, `unauthorized_client`, `expired_token`, …) as an enum
    instead of a string, so `app::login_hint` and `detect::DeviceCheck::Refused`
    match on it rather than on the error text.
  - Auth-mode advice out of the HTTP layer: `jenkins::send` / `login_page`
    append mode-specific hints ("use basic auth with an API token", oauth2-proxy
    flags); return a structured failure and add the hint where the mode is
    known (app / credentials). Do together with the item above.
  - Cache the OpenID Connect discovery (`Endpoints` per issuer) and one
    reqwest client in the executor, so a token refresh (every ~4 min with
    Keycloak's defaults) is one request, not client + discovery + token.
  - Maybe cache the basic-auth password between reconnects (keyring / `pass`
    prompt each time), dropped on a 401 or a password change. Careful:
    re-reading it is what picks up a changed token.
  - Save/Discard rows: their own (untitled) `Section` instead of
    `Section::Application`, so the UI needs no blank-line / dimming special
    cases for them.
  - Settings list scrolling: reuse `ui::visible_window` with an offset `Cell`
    like the other lists, instead of the stateless "selection two lines above
    the bottom" scroll in `render_settings`.
  - Fewer per-frame `Settings` clones: `effective()` / `edited()` /
    `connection_config()` clone and overlay everything (header, `theme()`,
    `rows()`); read single values from env-then-file instead. Microseconds,
    only as a tidy-up.
  - `AuthSettings::password` as an `auth::Secret` (with serde) instead of a
    `String` with a hand-written masking `Debug`.
