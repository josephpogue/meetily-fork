#!/bin/bash
# Pre-grant permissions before anything launches. Needs SIP off (the Cirrus base image has it off).
set -euo pipefail
now=$(date +%s)
# row <service> <client> <client_type: 0 bundle id, 1 path>
row() { echo "INSERT OR REPLACE INTO access VALUES('$1','$2',$3,2,4,1,NULL,NULL,0,'UNUSED',NULL,0,$now,NULL,NULL,'UNUSED',$now);"; }
user_db="$HOME/Library/Application Support/com.apple.TCC/TCC.db"
sys_db="/Library/Application Support/com.apple.TCC/TCC.db"

# Meetily itself.
sqlite3 "$user_db" "$(row kTCCServiceMicrophone com.meetily.ai 0) $(row kTCCServiceAudioCapture com.meetily.ai 0)"
sudo sqlite3 "$sys_db" "$(row kTCCServiceScreenCapture com.meetily.ai 0)"

# The call stand-ins open the microphone like a call; without a row the first launch would ask.
sqlite3 "$user_db" "$(row kTCCServiceMicrophone us.zoom.xos 0) $(row kTCCServiceMicrophone com.microsoft.teams2 0)"

# `testenv shot` runs screencapture under tart-guest-agent (as admin, in the GUI session), so Sequoia treats the agent as
# the app capturing the screen and covers the screen with "tart-guest-agent is requesting to bypass the system private
# window picker". Grant it by path in TCC (the symlink and the Cellar file it points at) and pre-approve it in replayd's
# approvals file with a far-future privacy-hint date, so the one-month prompt never shows. An image for any app needs this for `shot`.
# Meetily needs the same pre-approval: once the assistant session opens, it captures the Zoom window, and Sequoia shows the
# same prompt for "meetily-fork". replayd keys an app by bundle id (it wrote "com.meetily.ai" when it showed the prompt).
# The approvals file is overwritten (any existing content is replaced) and replayd is restarted so it reads the new one.
agent_link=/opt/homebrew/bin/tart-guest-agent
agent_real=$(readlink -f "$agent_link")
[ -x "$agent_real" ] || { echo "tcc.sh: $agent_link does not resolve to a program (got '$agent_real')" >&2; exit 1; }
sudo sqlite3 "$sys_db" "$(row kTCCServiceScreenCapture "$agent_link" 1) $(row kTCCServiceScreenCapture "$agent_real" 1)"
approvals="$HOME/Library/Group Containers/group.com.apple.replayd/ScreenCaptureApprovals.plist"
mkdir -p "$(dirname "$approvals")"
stamp=$(date -u +%Y-%m-%dT%H:%M:%SZ)
{
  echo '<?xml version="1.0" encoding="UTF-8"?>'
  echo '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">'
  echo '<plist version="1.0"><dict>'
  for p in "$agent_link" "$agent_real" com.meetily.ai; do
    echo "<key>$p</key><dict>"
    echo '<key>kScreenCaptureAlertableUsageCount</key><integer>1</integer>'
    echo "<key>kScreenCaptureApprovalLastAlerted</key><date>$stamp</date>"
    echo "<key>kScreenCaptureApprovalLastUsed</key><date>$stamp</date>"
    echo '<key>kScreenCapturePrivacyHintDate</key><date>2099-01-01T00:00:00Z</date>'
    echo '<key>kScreenCapturePrivacyHintPolicy</key><integer>2592000</integer>'
    echo '</dict>'
  done
  echo '</dict></plist>'
} > "$approvals"
chmod 600 "$approvals"
plutil -lint "$approvals"
killall replayd 2>/dev/null || true  # replayd reads the file when it starts; launchd restarts it on demand

sqlite3 "$user_db" "SELECT service FROM access WHERE client='com.meetily.ai';"
sudo sqlite3 "$sys_db" "SELECT service, client, client_type FROM access WHERE client='com.meetily.ai' OR client LIKE '%tart-guest-agent';"

# Notifications from Meetily as banners with no "allow notifications" prompt. macOS 15 keeps the grant in the user's
# com.apple.ncprefs.plist (usernoted reads it; the usernoted db2 file holds no grant). An entry with Messages' flags
# value 0x21788000e (copied from Messages, which is allowed as banners) makes UNUserNotificationCenter report
# authorizationStatus 2 (authorized), alertStyle 1 (banner); proven in a probe app with this bundle id, which showed a
# banner and no prompt. A bare entry with other flags still asked, and after the prompt timed out it recorded a denial.
# The path is not matched against the app, but usernoted deletes an entry at boot when its path does not exist (proven:
# an entry naming a missing /Applications/meetily-fork.app was gone after a reboot, the same entry naming an existing app
# survived), and the real app is copied in only after boot. So the entry names an app that is in the image, the Zoom
# stand-in; usernoted rewrites the path to the real app the first time Meetily posts.
python3 - <<'PY'
import os, plistlib
p = os.path.expanduser("~/Library/Preferences/com.apple.ncprefs.plist")
d = plistlib.load(open(p, "rb"))
d["apps"] = [a for a in d["apps"] if a.get("bundle-id") != "com.meetily.ai"]
d["apps"].append({"auth": 23, "bundle-id": "com.meetily.ai", "content_visibility": 0, "flags": 0x21788000E,
                  "grouping": 0, "path": "/Applications/Zoom Stand-in.app", "src": [{"flags": 6}]})
plistlib.dump(d, open(p, "wb"), fmt=plistlib.FMT_BINARY)
PY
killall usernoted cfprefsd 2>/dev/null || true  # both cache the file; launchd restarts them
plutil -p "$HOME/Library/Preferences/com.apple.ncprefs.plist" | grep -A3 '"auth" => 23' | grep -c com.meetily.ai
