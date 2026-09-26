# Leeroy
Jenkins TUI

## Configuration

Settings can be edited in the TUI (press `s`) and are saved to a TOML file:

1. `--config FILE`, if given
2. else `$LEEROY_CONFIG`
3. else `$XDG_CONFIG_HOME/leeroy/config.toml` (`~/.config/leeroy/config.toml`
   when `$XDG_CONFIG_HOME` is unset)

An existing `~/.leeroy/config.toml` is still picked up when there's no file at
step 3; new configs are always created in the XDG location. If both exist,
Leeroy uses the XDG one and prints a warning to stderr.

```toml
[jenkins]
url = "https://jenkins.example.com"
username = "me"
token = "..."   # API token; the file is written with mode 0600
```

Every setting can be overridden by an env var, which wins over the file (and
can't be edited in the TUI while set):

| Setting  | Env var                   |
| -------- | ------------------------- |
| URL      | `LEEROY_JENKINS_URL`      |
| Username | `LEEROY_JENKINS_USERNAME` |
| Token    | `LEEROY_JENKINS_TOKEN`    |

Saving from the TUI keeps comments and unknown keys in the file.
