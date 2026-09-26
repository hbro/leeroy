# Screenshots for the README (960x540: a quarter of full HD), committed in
# docs/screenshots/. Regenerate with
# scripts/screenshot.sh readme after UI changes. Data: the fake Jenkins.
Set Theme "Catppuccin Mocha"
Set Width 960
Set Height 540
Source tapes/_common.tape
Output target/screenshots/readme.gif
# A made-up instance and user: the fake Jenkins doubles as a proxy for any
# host and echoes the basic-auth user (alice:token).
Env LEEROY_JENKINS_URL "http://jenkins.example.com"
Env LEEROY_PROXY_URL "http://127.0.0.1:8099"
Env LEEROY_JENKINS_HEADERS_AUTHORIZATION "Basic YWxpY2U6dG9rZW4="

Hide
Type "target/debug/leeroy" Enter
Sleep 1.5s
Down
Down
Show
Screenshot docs/screenshots/jobs.png
Sleep 500ms
Hide
Type "2"
Sleep 1s
Show
Screenshot docs/screenshots/builds.png
Sleep 500ms
Hide
Type "1"
Type "/release"
Enter
Enter
Sleep 1s
Show
Screenshot docs/screenshots/build.png
Sleep 500ms
Hide
Type "q"
Type "q"
