#!/bin/bash
# Start Meetily in a test Mac with the WebDriver port; open --env keeps the app its own responsible process for permissions.
open --env TAURI_WEBDRIVER_PORT=4445 --stdout /tmp/testenv-app.log --stderr /tmp/testenv-app.log -a /Applications/meetily-fork.app
