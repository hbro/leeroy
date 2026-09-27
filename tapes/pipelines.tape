Source tapes/_settings.tape
Output target/screenshots/pipelines.gif
Env LEEROY_JENKINS_URL "http://127.0.0.1:8099"

Hide
Type "target/debug/leeroy" Enter
Sleep 1.5s
Type "3"
Sleep 1s
Show
Screenshot target/screenshots/pipelines.png
Sleep 500ms
Hide
Type "b"
Sleep 300ms
Show
Screenshot target/screenshots/pipelines-start-prompt.png
Sleep 500ms
Hide
Type "n"
Enter
Down 2
Sleep 300ms
Show
Screenshot target/screenshots/pipelines-run-tree.png
Sleep 500ms
Hide
Type "v"
Sleep 300ms
Show
Screenshot target/screenshots/pipelines-run-boxes.png
Sleep 500ms
Hide
Left
Sleep 300ms
Show
Screenshot target/screenshots/pipelines-run-older.png
Sleep 500ms
Hide
Escape
Type "G"
Enter
Type "p"
Space
Sleep 300ms
Show
Screenshot target/screenshots/pipelines-promote.png
Sleep 500ms
Hide
Type "q"
Type "q"
