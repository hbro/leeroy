Source tapes/_settings.tape
Output target/screenshots/builds.gif
Env LEEROY_JENKINS_URL "http://127.0.0.1:8099"

Hide
Type "target/debug/leeroy" Enter
Sleep 1.5s
Type "2"
Sleep 1s
Down 2
Show
Screenshot target/screenshots/builds.png
Sleep 500ms
Hide
Type "q"
Type "q"
