# A light terminal scheme; the theme stays on auto, so this also shows that
# Leeroy detects the light background.
Set Theme "Catppuccin Latte"
Set Width 1200
Set Height 720
Source tapes/_common.tape
Output target/screenshots/light.gif
Env LEEROY_JENKINS_URL "http://127.0.0.1:8099"

Hide
Type "target/debug/leeroy" Enter
Sleep 1.5s
Down
Down
Show
Screenshot target/screenshots/light-jobs.png
Sleep 500ms
Hide
Type "/"
Type "release"
Sleep 300ms
Show
Screenshot target/screenshots/light-filter.png
Sleep 500ms
Hide
Enter
Enter
Sleep 800ms
Show
Screenshot target/screenshots/light-build-running.png
Sleep 500ms
Hide
Escape
Type "0"
Up 2
Sleep 300ms
Show
Screenshot target/screenshots/light-settings.png
Sleep 500ms
Hide
Type "?"
Sleep 300ms
Show
Screenshot target/screenshots/light-help.png
Sleep 500ms
Hide
Escape
Type "q"
Type "q"
