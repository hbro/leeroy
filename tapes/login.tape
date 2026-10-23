Source tapes/_settings.tape
Output target/screenshots/login.gif
Env LEEROY_JENKINS_URL "http://127.0.0.1:8099"
Env LEEROY_AUTH_MODE "oidc"
Env LEEROY_AUTH_ISSUER "http://127.0.0.1:8099/realms/fake"
Env LEEROY_AUTH_CLIENT_ID "leeroy"
Env LEEROY_AUTH_TOKEN_STORE "memory"

# Device code login: the fake approves it once its page is opened.
Hide
Type "target/debug/leeroy" Enter
Sleep 1.5s
Show
Screenshot target/screenshots/login.png
Sleep 500ms
Hide
Escape
Type "q"
Type "q"
