#!/bin/bash
# Pre-grant Meetily's permissions before it ever launches. Needs SIP off (the Cirrus base image has it off).
set -euo pipefail
now=$(date +%s)
row() { echo "INSERT OR REPLACE INTO access VALUES('$1','com.meetily.ai',0,2,4,1,NULL,NULL,0,'UNUSED',NULL,0,$now,NULL,NULL,'UNUSED',$now);"; }
user_db="$HOME/Library/Application Support/com.apple.TCC/TCC.db"
sys_db="/Library/Application Support/com.apple.TCC/TCC.db"
sqlite3 "$user_db" "$(row kTCCServiceMicrophone) $(row kTCCServiceAudioCapture)"
sudo sqlite3 "$sys_db" "$(row kTCCServiceScreenCapture)"
sqlite3 "$user_db" "SELECT service FROM access WHERE client='com.meetily.ai';"
sudo sqlite3 "$sys_db" "SELECT service FROM access WHERE client='com.meetily.ai';"
