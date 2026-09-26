Source tapes/_settings.tape
Output target/screenshots/headers.gif

Hide
Type "target/debug/leeroy" Enter
Sleep 1s
Type "s"
Enter
Type "http://127.0.0.1:8099"
Enter
Down 3
Enter
Type "X-Forwarded-User: me"
Enter
Down
Enter
Type "Authorization: Basic bWU6czNjcmV0"
Sleep 300ms
Show
Screenshot target/screenshots/headers.png
Sleep 500ms
Hide
Escape
Type "q"
