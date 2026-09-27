Source tapes/_settings.tape
Output target/screenshots/jobs.gif
Env LEEROY_JENKINS_URL "http://127.0.0.1:8099"

Hide
Type "target/debug/leeroy" Enter
Sleep 1.5s
Down
Down
Show
Screenshot target/screenshots/jobs.png
Sleep 500ms
Hide
Type "R"
Sleep 300ms
Show
Screenshot target/screenshots/jobs-auto-off.png
Sleep 500ms
Hide
Down 7
Sleep 300ms
Show
Screenshot target/screenshots/jobs-selected-not-built.png
Sleep 500ms
Hide
Type "/"
Type "main"
Sleep 300ms
Show
Screenshot target/screenshots/jobs-filter.png
Sleep 500ms
Hide
Escape
Type "q"
