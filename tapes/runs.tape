Source tapes/_settings.tape
Output target/screenshots/runs.gif
Env LEEROY_JENKINS_URL "http://127.0.0.1:8099"

Hide
Type "target/debug/leeroy" Enter
Sleep 1.5s
Type "4"
Sleep 1s
Down 2
Sleep 300ms
Show
Screenshot target/screenshots/runs.png
Sleep 500ms
Hide
Enter
Sleep 300ms
Show
Screenshot target/screenshots/runs-run.png
Sleep 500ms
Hide
Type "q"
Type "q"
