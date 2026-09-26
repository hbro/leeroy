Source tapes/_settings.tape
Output target/screenshots/console.gif
Env LEEROY_JENKINS_URL "http://127.0.0.1:8099"

Hide
Type "target/debug/leeroy" Enter
Sleep 1.5s
Type "/release"
Enter
Enter
Sleep 800ms
Type "c"
Sleep 1.5s
Show
Screenshot target/screenshots/console.png
Sleep 500ms
Hide
Up 5
Sleep 300ms
Show
Screenshot target/screenshots/console-paused.png
Sleep 500ms
Hide
Escape
Escape
Type "q"
Type "q"
