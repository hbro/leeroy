Source tapes/_settings.tape
Output target/screenshots/settings.gif

Hide
Type "target/debug/leeroy" Enter
Sleep 1s
Type "s"
Sleep 300ms
Show
Screenshot target/screenshots/settings.png
Sleep 500ms
Hide
Enter
Type "http://127.0.0.1:8099"
Sleep 300ms
Show
Screenshot target/screenshots/settings-editing.png
Sleep 500ms
Hide
Enter
Sleep 1s
Show
Screenshot target/screenshots/settings-saved.png
Sleep 500ms
Hide
Down 2
Enter
Type "socks5h://me:secret@bastion:1080"
Enter
Sleep 500ms
Show
Screenshot target/screenshots/settings-proxy.png
Sleep 500ms
Hide
Type "q"
