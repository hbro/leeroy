Source tapes/_settings.tape
Output target/screenshots/auth.gif
Env LEEROY_JENKINS_URL "http://127.0.0.1:8099"

# The basic-auth rows of the settings view.
Hide
Type "target/debug/leeroy" Enter
Sleep 1s
Type "0"
Down 3
Enter
Enter
Down 2
Enter
Type "me"
Enter
Ctrl+S
Down 2
Sleep 500ms
Show
Screenshot target/screenshots/auth.png
Sleep 500ms
Hide
Type "q"
Type "q"
