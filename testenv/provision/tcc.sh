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

# `testenv shot` runs screencapture under tart-guest-agent (as admin, in the GUI session), so Sequoia treats the agent as
# the app capturing the screen and covers the screen with "tart-guest-agent is requesting to bypass the system private
# window picker". Grant it by path in TCC (the symlink and the Cellar file it points at) and pre-approve it in replayd's
# approvals file with a far-future privacy-hint date, so the one-month prompt never shows. An image for any app needs this for `shot`.
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
  for p in "$agent_link" "$agent_real"; do
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
