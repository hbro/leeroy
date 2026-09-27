Source tapes/_settings.tape
Output target/screenshots/info.gif
Env LEEROY_JENKINS_URL "http://127.0.0.1:8099"

Hide
Type "target/debug/leeroy" Enter
Sleep 1.5s
Type "i"
Sleep 800ms
Show
Screenshot target/screenshots/info.png
Sleep 500ms
Hide
Type "i"
Type "q"
Type "q"
