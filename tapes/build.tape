Source tapes/_settings.tape
Output target/screenshots/build.gif
Env LEEROY_JENKINS_URL "http://127.0.0.1:8099"

Hide
Type "target/debug/leeroy" Enter
Sleep 1.5s
Enter
Sleep 800ms
Show
Screenshot target/screenshots/build.png
Sleep 500ms
Hide
Escape
Type "/release"
Enter
Enter
Sleep 800ms
Show
Screenshot target/screenshots/build-running.png
Sleep 500ms
Hide
Escape
Type "q"
