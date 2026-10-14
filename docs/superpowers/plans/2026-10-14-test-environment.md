# Test Environment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Before a Meetily PR opens, a user-mock agent installs the build on a fresh throwaway test Mac, uses it, and reports pass or fail per step.

**Architecture:** A shared `testenv` command (new repo) wraps Tart to create, drive and delete throwaway macOS VMs cloned from a prepared image. The prepared image is built by Packer from a template in the app's repo. The `user-mock` agent drives the app through an embedded WebDriver server that only test builds contain; dev-lead starts user-mock instead of walking the app itself.

**Tech Stack:** bash, Tart 2.41.0, Packer with `packer-plugin-tart` >= 1.21.0, bats-core, WebdriverIO (`webdriverio` npm, standalone `remote()`), `tauri-plugin-wdio-webdriver` 1.5, Swift (`swiftc`, Xcode 26.3), sox, BlackHole 2ch, launchd, runlog.

**Spec:** `docs/superpowers/specs/2026-10-14-test-environment-design.md` (this worktree).

## Global Constraints

- Tart binary: `~/.local/opt/tart-2.41.0/tart.app/Contents/MacOS/tart`. Never the Homebrew tap.
- Base image: `ghcr.io/cirruslabs/macos-sequoia-base:latest` (already cached, 33 GB). Guest login `admin` / `admin`; Homebrew, node 24 and pnpm are already in it; SIP is off.
- At most 2 macOS VMs running at once (Apple license). `up` waits up to 600 s for a slot, then fails.
- Every test Mac runs with `--no-audio` and `--no-graphics`. The real host microphone never reaches a guest.
- Guest audio: BlackHole 2ch is the fake microphone (default input); BlackHole 16ch is the fake speakers (default output). Two devices, so call audio played as Them never reaches the microphone.
- Exit code 2 means the environment failed. Every other non-zero code comes from the command that ran.
- Run VM names start with `run-`. Prepared images are named `<app>-golden`. Reaper TTL: 7200 s.
- Run output: `~/.cache/testenv/runs/<id>/`, kept after `down`.
- The WebDriver add-on exists only in builds made with the cargo feature `testenv`. Joseph's installed release build never has it.
- Permission rows: user TCC.db gets `kTCCServiceMicrophone` and `kTCCServiceAudioCapture` for `com.meetily.ai`; system TCC.db gets `kTCCServiceScreenCapture` for `com.meetily.ai`. client_type 0, auth_value 2, csreq NULL.
- No Claude login, API key or token is ever copied into a guest.
- Commits: concise, high-level, no AI co-author lines. No em dashes in docs written for Joseph.
- Personal-repo flow from `~/.claude/CLAUDE.md`: worktree, `test-gate run -- <suite>` before `gh pr create`, PR to `main`.

## Review Focus

1. A third `testenv up` while two run: it must wait, then exit 2 with "no free slot", never start a third VM. (Task 2 test.)
2. `up` after `golden.pkr.hcl` or any `testenv/` file changed: it must refuse with exit 2 and say to run `bake`. (Task 2 test.)
3. `up` fails or is stopped midway (VM cloned, never ready): `up` deletes the half-made `run-*` VM itself; if `up` was killed outright, `ls` shows the VM and the reaper deletes it once it is two hours old. (Task 2 and Task 3 tests.)
4. The app never reaches its window (crash at launch, like the onnxruntime bug): `ui.mjs wait` must time out with a clear message and `collect` must still copy the crash report. (Task 9 test and Task 12 step.)
5. A permission prompt appears anyway (stale rows after a TCC change): user-mock must report it as a failed "no permission prompt" step with a screenshot, not get stuck. (Task 10 instructions, Task 12 check.)

---

## Part A: the `testenv` repo

Repo path: `~/brain/departments/personal/projects/testenv`, remote `github.com/josephpogue/testenv` (private).

File map:
- `bin/testenv`: entry point, dispatches subcommands.
- `lib/common.sh`: paths, `tart` wrapper, `die_env` (exit 2), id and run-dir helpers.
- `lib/vm.sh`: `up`, `down`, `exec`, `ls`, `reap`, slot and stale-image checks.
- `lib/media.sh`: `shot`, `play`, `collect`.
- `lib/bake.sh`: `bake` (Packer) and the recipe hash.
- `test/stubs/tart`, `test/stubs/packer`: fakes driven by files in `$STUB_STATE`.
- `test/*.bats`: unit tests against the stubs.
- `launchd/com.josephpogue.testenv-reap.plist`, `install.sh`: link the command and load the cleanup job.
- `README.md`, `CLAUDE.md`.

### Task 1: Repo skeleton, stub tart, and `ls`

**Files:**
- Create: `bin/testenv`, `lib/common.sh`, `lib/vm.sh`, `test/stubs/tart`, `test/helper.bash`, `test/ls.bats`, `Makefile`, `.gitignore`

**Interfaces:**
- Produces: `TART` (path, overridable by env `TESTENV_TART`), `RUNS_DIR` (`${TESTENV_RUNS:-$HOME/.cache/testenv/runs}`), `die_env <msg>` (prints `testenv: <msg>` to stderr, exits 2), `new_id <app>` (prints `<app>-<yyyymmdd-HHMMSS>-<4 hex>`), `cmd_ls`.
- Stub contract: `test/stubs/tart` reads and writes `$STUB_STATE/vms` (lines `name state created_epoch`), appends every call to `$STUB_STATE/calls`.

- [ ] **Step 1: Create the repo and worktree**

```bash
mkdir -p ~/brain/departments/personal/projects/testenv && cd ~/brain/departments/personal/projects/testenv
git init -b main && git commit --allow-empty -m "Start testenv"
gh repo create josephpogue/testenv --private --source . --push
git worktree add ../testenv-worktrees/initial -b feat/initial origin/main
cd ../testenv-worktrees/initial && git commit --allow-empty -m "Begin initial build"
brew install bats-core packer shellcheck
```

- [ ] **Step 2: Write the stub tart**

`test/stubs/tart`:
```bash
#!/bin/bash
# Fake tart for unit tests. State lives in $STUB_STATE/vms as "name state created_epoch".
set -euo pipefail
S="${STUB_STATE:?}"; touch "$S/vms" "$S/calls"
echo "$*" >> "$S/calls"
cmd="$1"; shift
case "$cmd" in
  list)
    echo "Source Name State"
    while read -r n st _; do echo "local $n $st"; done < "$S/vms" ;;
  clone)
    [ -f "$S/fail_clone" ] && exit 1
    echo "$2 stopped $(date +%s)" >> "$S/vms" ;;
  run)
    name="${@: -1}"
    sed -i '' "s/^$name stopped/$name running/" "$S/vms"
    sleep "${STUB_RUN_SLEEP:-0}" ;;
  exec)
    [ -f "$S/exec_fail" ] && exit 1
    echo "stub-exec $*" ;;
  stop)   sed -i '' "s/^$1 running/$1 stopped/" "$S/vms" ;;
  delete) grep -v "^$1 " "$S/vms" > "$S/vms.new" || true; mv "$S/vms.new" "$S/vms" ;;
  get)    echo '{"Running": true}' ;;
  ip)     echo 192.168.64.10 ;;
  *) echo "stub: unknown $cmd" >&2; exit 64 ;;
esac
```

`test/helper.bash`:
```bash
setup() {
  export STUB_STATE="$BATS_TEST_TMPDIR/state"; mkdir -p "$STUB_STATE"
  export TESTENV_TART="$BATS_TEST_DIRNAME/stubs/tart"
  export TESTENV_RUNS="$BATS_TEST_TMPDIR/runs"
  export TESTENV_SLOT_WAIT=1 TESTENV_READY_WAIT=2
  TESTENV="$BATS_TEST_DIRNAME/../bin/testenv"
}
```

- [ ] **Step 3: Write the failing test**

`test/ls.bats`:
```bash
load helper

@test "ls shows run VMs with age and hides other VMs" {
  now=$(date +%s)
  printf 'run-meetily-fork-a stopped %s\nmeetily-fork-golden stopped %s\n' "$((now-120))" "$now" > "$STUB_STATE/vms"
  mkdir -p "$TESTENV_RUNS/meetily-fork-a"; echo "$((now-120))" > "$TESTENV_RUNS/meetily-fork-a/created"
  run "$TESTENV" ls
  [ "$status" -eq 0 ]
  [[ "$output" == *"run-meetily-fork-a"* ]]
  [[ "$output" != *"meetily-fork-golden"* ]]
}
```

- [ ] **Step 4: Run it to see it fail**

Run: `bats test/ls.bats`
Expected: FAIL, `bin/testenv: No such file or directory`.

- [ ] **Step 5: Write the minimal code**

`lib/common.sh`:
```bash
# Shared helpers for testenv.
TART="${TESTENV_TART:-$HOME/.local/opt/tart-2.41.0/tart.app/Contents/MacOS/tart}"
RUNS_DIR="${TESTENV_RUNS:-$HOME/.cache/testenv/runs}"
MAX_VMS=2
TTL_SECONDS="${TESTENV_TTL:-7200}"

die_env() { echo "testenv: $*" >&2; exit 2; }
new_id() { echo "$1-$(date +%Y%m%d-%H%M%S)-$(openssl rand -hex 2)"; }
run_dir() { echo "$RUNS_DIR/$1"; }
# Creation time of a run VM, from the marker up writes before cloning.
created_at() { cat "$(run_dir "${1#run-}")/created" 2>/dev/null || echo 0; }
run_vms() { "$TART" list 2>/dev/null | awk 'NR>1 && $2 ~ /^run-/ {print $2, $NF}'; }
```

`lib/vm.sh`:
```bash
cmd_ls() {
  local now; now=$(date +%s)
  run_vms | while read -r name state; do
    printf '%-48s %-8s %ss\n' "$name" "$state" "$(( now - $(created_at "$name") ))"
  done
}
```

`bin/testenv`:
```bash
#!/bin/bash
# testenv: throwaway macOS test VMs. Exit 2 = the environment failed.
set -euo pipefail
here="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
. "$here/lib/common.sh"
for f in "$here"/lib/*.sh; do [ "$f" = "$here/lib/common.sh" ] || . "$f"; done
sub="${1:-}"; shift || true
case "$sub" in
  ls|up|down|exec|reap|shot|play|collect|bake) "cmd_$sub" "$@" ;;
  *) echo "usage: testenv {bake|up|exec|shot|play|collect|down|ls|reap}" >&2; exit 64 ;;
esac
```

`Makefile`:
```make
test: ; bats test
lint: ; shellcheck bin/testenv lib/*.sh test/stubs/*
```

`.gitignore`: `*.log`

- [ ] **Step 6: Run the tests**

Run: `chmod +x bin/testenv test/stubs/tart && bats test/ls.bats`
Expected: PASS (1 test).

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "Add testenv skeleton and ls"
```

### Task 2: `up` with slot limit and stale-image check

**Files:**
- Modify: `lib/vm.sh`
- Create: `test/up.bats`

**Interfaces:**
- Consumes: `die_env`, `new_id`, `run_dir`, `run_vms`, `TART`.
- Consumes from Task 6: the image hash file `~/.cache/testenv/images/<app>-golden.hash` written by `bake` (kept outside Tart's own VM folder so Tart never sees a stray file). Override with env `TESTENV_HASH_FILE` in tests.
- Produces: `cmd_up <app> --app <path> --recipe <dir>` prints the id on stdout and nothing else; writes `run_dir/<id>/{created,app_path,vm.pid}`. `recipe_hash <dir>` prints a sha256 of every file under `<dir>` sorted by path.

- [ ] **Step 1: Write the failing tests**

`test/up.bats`:
```bash
load helper

seed_golden() {
  mkdir -p "$BATS_TEST_TMPDIR/recipe" && echo a > "$BATS_TEST_TMPDIR/recipe/golden.pkr.hcl"
  echo "meetily-fork-golden stopped 0" >> "$STUB_STATE/vms"
  export TESTENV_HASH_FILE="$BATS_TEST_TMPDIR/hash"
  ( . "$BATS_TEST_DIRNAME/../lib/common.sh"; . "$BATS_TEST_DIRNAME/../lib/vm.sh"; recipe_hash "$BATS_TEST_TMPDIR/recipe" ) > "$TESTENV_HASH_FILE"
  mkdir -p "$BATS_TEST_TMPDIR/app/meetily-fork.app"
}

@test "up clones golden, starts with no audio, copies the app, prints the id" {
  seed_golden
  run "$TESTENV" up meetily-fork --app "$BATS_TEST_TMPDIR/app/meetily-fork.app" --recipe "$BATS_TEST_TMPDIR/recipe"
  [ "$status" -eq 0 ]
  [[ "$output" =~ ^meetily-fork-[0-9]{8}-[0-9]{6}-[0-9a-f]{4}$ ]]
  grep -q "^clone meetily-fork-golden run-$output" "$STUB_STATE/calls"
  grep -q -- "--no-audio" "$STUB_STATE/calls"
  grep -q -- "--no-graphics" "$STUB_STATE/calls"
}

@test "up refuses a stale image with exit 2" {
  seed_golden
  echo changed > "$BATS_TEST_TMPDIR/recipe/golden.pkr.hcl"
  run "$TESTENV" up meetily-fork --app "$BATS_TEST_TMPDIR/app/meetily-fork.app" --recipe "$BATS_TEST_TMPDIR/recipe"
  [ "$status" -eq 2 ]
  [[ "$output" == *"run: testenv bake meetily-fork"* ]]
}

@test "up with two VMs running waits, then exits 2 without a third clone" {
  seed_golden
  printf 'run-x-1 running 0\nrun-x-2 running 0\n' >> "$STUB_STATE/vms"
  run "$TESTENV" up meetily-fork --app "$BATS_TEST_TMPDIR/app/meetily-fork.app" --recipe "$BATS_TEST_TMPDIR/recipe"
  [ "$status" -eq 2 ]
  [[ "$output" == *"no free slot"* ]]
  ! grep -q "^clone" "$STUB_STATE/calls"
}

@test "up exits 2 when the guest never answers" {
  seed_golden
  touch "$STUB_STATE/exec_fail"
  run "$TESTENV" up meetily-fork --app "$BATS_TEST_TMPDIR/app/meetily-fork.app" --recipe "$BATS_TEST_TMPDIR/recipe"
  [ "$status" -eq 2 ]
  [[ "$output" == *"did not start"* ]]
  ! grep -q '^run-' "$STUB_STATE/vms"
}

@test "up deletes the half-made VM when the clone fails" {
  seed_golden
  touch "$STUB_STATE/fail_clone"
  run "$TESTENV" up meetily-fork --app "$BATS_TEST_TMPDIR/app/meetily-fork.app" --recipe "$BATS_TEST_TMPDIR/recipe"
  [ "$status" -eq 2 ]
  ! grep -q '^run-' "$STUB_STATE/vms"
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `bats test/up.bats`
Expected: 5 FAIL, `cmd_up: command not found`.

- [ ] **Step 3: Write the code**

Append to `lib/vm.sh`:
```bash
recipe_hash() { (cd "$1" && find . -type f ! -name '.*' -print0 | sort -z | xargs -0 shasum -a 256 | shasum -a 256 | cut -c1-16); }
hash_file() { echo "${TESTENV_HASH_FILE:-$HOME/.cache/testenv/images/$1-golden.hash}"; }
running_count() { run_vms | awk '$2=="running"' | wc -l | tr -d ' '; }

wait_for_slot() {
  local waited=0 limit="${TESTENV_SLOT_WAIT:-600}"
  while [ "$(running_count)" -ge "$MAX_VMS" ]; do
    [ "$waited" -ge "$limit" ] && die_env "no free slot: $MAX_VMS test Macs already running (Apple's limit)"
    sleep 5; waited=$((waited + 5))
  done
}

cmd_up() {
  local app="$1"; shift; local app_path="" recipe=""
  while [ $# -gt 0 ]; do case "$1" in --app) app_path="$2"; shift 2;; --recipe) recipe="$2"; shift 2;; *) die_env "unknown option $1";; esac; done
  [ -d "$app_path" ] || die_env "app not found: $app_path"
  [ -d "$recipe" ] || die_env "recipe folder not found: $recipe"
  [ "$(cat "$(hash_file "$app")" 2>/dev/null)" = "$(recipe_hash "$recipe")" ] || die_env "$app-golden is missing or older than $recipe; run: testenv bake $app"
  wait_for_slot
  # UP_ID and UP_OK are global so the EXIT trap still sees them after die_env exits.
  UP_ID=$(new_id "$app"); UP_OK=0
  trap '[ "$UP_OK" = 1 ] || cmd_down "$UP_ID" >/dev/null 2>&1 || true' EXIT
  local id="$UP_ID" vm="run-$UP_ID" dir; dir=$(run_dir "$id")
  mkdir -p "$dir"; date +%s > "$dir/created"; echo "$app_path" > "$dir/app_path"
  "$TART" clone "$app-golden" "$vm" >/dev/null 2>&1 || die_env "clone failed for $vm"
  nohup "$TART" run --no-graphics --no-audio --dir="build:$(dirname "$app_path"):ro" "$vm" > "$dir/tart-run.log" 2>&1 &
  echo $! > "$dir/vm.pid"
  local waited=0
  until "$TART" exec "$vm" true >/dev/null 2>&1; do
    [ "$waited" -ge "${TESTENV_READY_WAIT:-120}" ] && die_env "test Mac $vm did not start in time"
    sleep 1; waited=$((waited + 1))
  done
  local name; name=$(basename "$app_path")
  "$TART" exec "$vm" ditto "/Volumes/My Shared Files/build/$name" "/Applications/$name" >/dev/null || die_env "copying $name into $vm failed"
  UP_OK=1; echo "$id"
}
```

`cmd_down` is defined in Task 3; until then the trap's `|| true` keeps it harmless, and the two cleanup tests pass only once Task 3 lands. Run Task 2's tests with `bats test/up.bats -f 'clones|stale|two VMs'` here.

- [ ] **Step 4: Run the tests**

Run: `bats test/ls.bats && bats test/up.bats -f 'clones|stale|two VMs'`
Expected: PASS (4 tests). The two cleanup tests run green after Task 3.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "Add up with slot limit and stale-image check"
```

### Task 3: `exec`, `down` and `reap`

**Files:**
- Modify: `lib/vm.sh`
- Create: `test/down-reap.bats`

**Interfaces:**
- Produces: `cmd_exec <id> -- <cmd...>` (exit code of the command; 2 if the VM is gone), `cmd_down <id>` (stop, delete, keep run dir), `cmd_reap` (deletes every `run-*` older than `TTL_SECONDS`, including stopped half-made ones; prints one line per deletion).

- [ ] **Step 1: Write the failing tests**

`test/down-reap.bats`:
```bash
load helper

mk_run() { # name age_seconds state
  mkdir -p "$TESTENV_RUNS/${1#run-}"; echo "$(( $(date +%s) - $2 ))" > "$TESTENV_RUNS/${1#run-}/created"
  echo "$1 $3 0" >> "$STUB_STATE/vms"
}

@test "down deletes the VM and keeps the run folder" {
  mk_run run-a 10 running
  run "$TESTENV" down a
  [ "$status" -eq 0 ]
  ! grep -q '^run-a ' "$STUB_STATE/vms"
  [ -f "$TESTENV_RUNS/a/created" ]
}

@test "exec passes the command's exit code through" {
  mk_run run-a 10 running
  run "$TESTENV" exec a -- echo hi
  [ "$status" -eq 0 ]
  [[ "$output" == *"echo hi"* ]]
}

@test "exec on a missing VM exits 2" {
  run "$TESTENV" exec nope -- true
  [ "$status" -eq 2 ]
}

@test "reap deletes old run VMs, running or half-made, and keeps young ones and goldens" {
  mk_run run-old 8000 running
  mk_run run-half 8000 stopped
  mk_run run-young 60 running
  echo "meetily-fork-golden stopped 0" >> "$STUB_STATE/vms"
  run "$TESTENV" reap
  [ "$status" -eq 0 ]
  ! grep -q '^run-old ' "$STUB_STATE/vms"
  ! grep -q '^run-half ' "$STUB_STATE/vms"
  grep -q '^run-young ' "$STUB_STATE/vms"
  grep -q '^meetily-fork-golden ' "$STUB_STATE/vms"
}

@test "reap treats a run VM with no marker as old" {
  echo "run-orphan stopped 0" >> "$STUB_STATE/vms"
  run "$TESTENV" reap
  ! grep -q '^run-orphan ' "$STUB_STATE/vms"
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `bats test/down-reap.bats`
Expected: FAIL, `cmd_down: command not found`.

- [ ] **Step 3: Write the code**

Append to `lib/vm.sh`:
```bash
vm_exists() { run_vms | awk '{print $1}' | grep -qx "run-$1"; }

cmd_exec() {
  local id="$1"; shift; [ "${1:-}" = "--" ] && shift
  vm_exists "$id" || die_env "no test Mac run-$id"
  "$TART" exec "run-$id" "$@"
}

cmd_down() {
  local id="$1"; vm_exists "$id" || return 0
  "$TART" stop "run-$id" >/dev/null 2>&1 || true
  "$TART" delete "run-$id" >/dev/null 2>&1 || die_env "could not delete run-$id"
}

cmd_reap() {
  local now; now=$(date +%s)
  run_vms | while read -r name _; do
    local age=$(( now - $(created_at "$name") ))
    if [ "$age" -ge "$TTL_SECONDS" ]; then
      "$TART" stop "$name" >/dev/null 2>&1 || true
      "$TART" delete "$name" >/dev/null 2>&1 && echo "reaped $name (age ${age}s)"
    fi
  done
}
```

- [ ] **Step 4: Run the tests**

Run: `bats test`
Expected: PASS (11 tests), including the two `up` cleanup tests from Task 2.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "Add exec, down and reap"
```

### Task 4: `shot`, `play` and `collect`

**Files:**
- Create: `lib/media.sh`, `test/media.bats`

**Interfaces:**
- Consumes: `cmd_exec`, `run_dir`.
- Produces:
  - `cmd_shot <id> <name>`: runs `screencapture -x /tmp/<name>.png` in the guest, copies it to `run_dir/<id>/shots/<name>.png`, prints that host path.
  - `cmd_play <id> mic|call <guest-path>`: `mic` runs `sox <clip> -t coreaudio "BlackHole 2ch"` in the guest; `call` runs `afplay <clip>`. Blocks until the clip ends.
  - `cmd_collect <id>`: copies `~/Library/Logs/DiagnosticReports/*`, `~/Library/Logs/com.meetily.ai/*` and `/tmp/testenv-app.log` from the guest into `run_dir/<id>/collected/`.
- Copy-out mechanism: the guest writes into its own `/tmp`, then the host reads it with `tart exec <vm> cat <file> > <host file>`. No shared write folder is needed.

- [ ] **Step 1: Write the failing tests**

`test/media.bats`:
```bash
load helper

@test "shot saves into the run folder and prints the path" {
  echo "run-a running 0" >> "$STUB_STATE/vms"; mkdir -p "$TESTENV_RUNS/a"
  run "$TESTENV" shot a step1
  [ "$status" -eq 0 ]
  [ "$output" = "$TESTENV_RUNS/a/shots/step1.png" ]
  grep -q "exec run-a screencapture -x /tmp/step1.png" "$STUB_STATE/calls"
}

@test "play mic routes the clip into BlackHole" {
  echo "run-a running 0" >> "$STUB_STATE/vms"
  run "$TESTENV" play a mic /Users/admin/clips/you.wav
  [ "$status" -eq 0 ]
  grep -q 'sox /Users/admin/clips/you.wav -t coreaudio BlackHole 2ch' "$STUB_STATE/calls"
}

@test "play rejects an unknown route with exit 2" {
  echo "run-a running 0" >> "$STUB_STATE/vms"
  run "$TESTENV" play a speakers x.wav
  [ "$status" -eq 2 ]
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `bats test/media.bats`
Expected: FAIL, `cmd_shot: command not found`.

- [ ] **Step 3: Write the code**

`lib/media.sh`:
```bash
# Screenshots, audio and log collection inside a test Mac.
copy_out() { # id guest_path host_path
  mkdir -p "$(dirname "$3")"; "$TART" exec "run-$1" cat "$2" > "$3"
}

cmd_shot() {
  local id="$1" name="$2" out; out="$(run_dir "$id")/shots/$name.png"
  cmd_exec "$id" -- screencapture -x "/tmp/$name.png"
  copy_out "$id" "/tmp/$name.png" "$out"; echo "$out"
}

cmd_play() {
  local id="$1" route="$2" clip="$3"
  case "$route" in
    mic)  cmd_exec "$id" -- /opt/homebrew/bin/sox "$clip" -t coreaudio "BlackHole 2ch" ;;
    call) cmd_exec "$id" -- afplay "$clip" ;;
    *) die_env "play route must be mic or call, got $route" ;;
  esac
}

cmd_collect() {
  local id="$1" dest; dest="$(run_dir "$id")/collected"; mkdir -p "$dest"
  cmd_exec "$id" -- /bin/bash -c 'cd /tmp && tar czf testenv-collect.tgz -C / Users/admin/Library/Logs/DiagnosticReports Users/admin/Library/Logs/com.meetily.ai tmp/testenv-app.log 2>/dev/null; true'
  copy_out "$id" /tmp/testenv-collect.tgz "$dest/collect.tgz"
  tar xzf "$dest/collect.tgz" -C "$dest" 2>/dev/null || true
  echo "$dest"
}
```

The stub `exec` echoes its arguments, so in tests `copy_out` writes that echo into the file, which is enough.

- [ ] **Step 4: Run the tests**

Run: `bats test`
Expected: PASS (14 tests).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "Add shot, play and collect"
```

### Task 5: Cleanup job, install, docs, first PR

**Files:**
- Create: `launchd/com.josephpogue.testenv-reap.plist`, `bin/testenv-reap`, `install.sh`, `README.md`, `CLAUDE.md`, `test/reap-job.bats`
- Modify: `~/brain/departments/personal/projects/My-Life/automations/context-router/overrides.json` (in a My-Life worktree, its own PR)

**Interfaces:**
- Produces: `~/.local/bin/testenv` symlink; launchd label `com.josephpogue.testenv-reap`, every 900 s; runlog skill name `testenv-reap`; Mission Control manifest `~/.claude/automations.d/testenv-reap.json`.
- Mission Control lists an automation only from a manifest in `~/.claude/automations.d/`; a launchd job or run-log folder with no manifest shows as "needs registering" (`mission-control/Claude Code/lib/health/untracked.ts`). The manifest `name` must equal the runlog skill name.

- [ ] **Step 1: Write the failing test**

`test/reap-job.bats`:
```bash
load helper

@test "reap job opens and seals a runlog run" {
  export RUNLOG_BIN="$BATS_TEST_TMPDIR/runlog"; : > "$BATS_TEST_TMPDIR/runlog.calls"
  printf '#!/bin/bash\necho "$*" >> %s\n[ "$1" = start ] && echo rid-1\n[ "$1" = guard ] && echo true\nexit 0\n' "$BATS_TEST_TMPDIR/runlog.calls" > "$RUNLOG_BIN"; chmod +x "$RUNLOG_BIN"
  run "$BATS_TEST_DIRNAME/../bin/testenv-reap"
  [ "$status" -eq 0 ]
  grep -q '^start testenv-reap' "$BATS_TEST_TMPDIR/runlog.calls"
  grep -q '^done rid-1' "$BATS_TEST_TMPDIR/runlog.calls"
}
```

- [ ] **Step 2: Run it to see it fail**

Run: `bats test/reap-job.bats`
Expected: FAIL, no such file `bin/testenv-reap`.

- [ ] **Step 3: Write the job, plist and installer**

`bin/testenv-reap`:
```bash
#!/bin/bash
# launchd entry: delete test Macs past their TTL, logged to Mission Control.
set -euo pipefail
RUNLOG="${RUNLOG_BIN:-$HOME/.local/bin/runlog}"
here="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
rid=$("$RUNLOG" start testenv-reap --trigger "${RUNLOG_TRIGGER:-scheduled}")
eval "$("$RUNLOG" guard "$rid")"
out=$("$here/testenv" reap)
"$RUNLOG" done "$rid" --result "${out:-nothing to reap}"
```

`launchd/com.josephpogue.testenv-reap.plist`:
```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>com.josephpogue.testenv-reap</string>
  <key>ProgramArguments</key><array><string>/Users/joseph.pogue/.local/bin/testenv-reap</string></array>
  <key>StartInterval</key><integer>900</integer>
  <key>EnvironmentVariables</key><dict>
    <key>PATH</key><string>/Users/joseph.pogue/.local/bin:/opt/homebrew/bin:/usr/bin:/bin</string>
    <key>RUNLOG_TRIGGER</key><string>scheduled</string>
  </dict>
  <key>StandardErrorPath</key><string>/Users/joseph.pogue/.cache/testenv/reap.err.log</string>
</dict></plist>
```

`install.sh`:
```bash
#!/bin/bash
# Link testenv into ~/.local/bin and load the cleanup job. Run from the main checkout.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
mkdir -p ~/.local/bin ~/.cache/testenv ~/Library/LaunchAgents
ln -sf "$here/bin/testenv" ~/.local/bin/testenv
ln -sf "$here/bin/testenv-reap" ~/.local/bin/testenv-reap
cp "$here/launchd/com.josephpogue.testenv-reap.plist" ~/Library/LaunchAgents/
launchctl bootout "gui/$(id -u)/com.josephpogue.testenv-reap" 2>/dev/null || true
launchctl bootstrap "gui/$(id -u)" ~/Library/LaunchAgents/com.josephpogue.testenv-reap.plist
```

- [ ] **Step 4: Run all tests and lint**

Run: `chmod +x bin/testenv-reap install.sh && make test lint`
Expected: PASS (15 tests), shellcheck clean.

- [ ] **Step 5: Write README.md and CLAUDE.md**

`README.md`: one table of the commands from the spec's Part 1, the exit-code rule, the two-VM rule, where run output goes, how to add an app (a `testenv/` folder with `golden.pkr.hcl`), and `./install.sh`.
`CLAUDE.md`: Goal (one prose block, from the spec Goal), then: tests are `make test` (bats, stubs only, no real VM); `test-gate run -- make test`; never raise `MAX_VMS`; exit 2 is reserved for environment failures; run output lives outside the repo.

- [ ] **Step 6: Commit, gate, PR**

```bash
git add -A && git commit -m "Add cleanup job, installer and docs"
test-gate run -- make test
git push -u origin feat/initial && gh pr create --base main --title "testenv: throwaway macOS test VMs" --body "Commands, slot limit, stale-image check, cleanup job. Tests: make test (15, stubbed)."
```

- [ ] **Step 7: Register the repo with the router**

In a My-Life worktree, add to `automations/context-router/overrides.json`, beside the meetily-fork entry:
```json
"~/brain/departments/personal/projects/testenv": {
  "department": "personal",
  "note": "throwaway macOS test VMs for app testing (Tart). Remote github.com/josephpogue/testenv"
},
```
Run `bash automations/context-router/build-registry.sh`, confirm `grep testenv ~/.claude/routing/registry.md` prints the row, commit, gate, PR.

- [ ] **Step 8: Register the card in Mission Control**

Write `~/.claude/automations.d/testenv-reap.json` (same shape as `worktree-sweep.json`; Mission Control's cadence list has no 15-minute value, so `daily` is the closest one that still flags a job that stopped running):
```json
{
  "name": "testenv-reap",
  "label": "Test Mac Cleanup",
  "description": "Every 15 minutes, deletes throwaway test Macs older than two hours so a crashed test run never leaves a VM behind.",
  "location": "~/brain/departments/personal/projects/testenv",
  "trigger": "launchd:com.josephpogue.testenv-reap",
  "cadence": "daily",
  "category": "AI & Agents"
}
```
Commit it in `~/.claude`: `git -C ~/.claude add automations.d/testenv-reap.json && git -C ~/.claude commit -m "Register the test Mac cleanup job"`.

- [ ] **Step 9: After merge, install and check the card**

Run in the main checkout: `git pull && ./install.sh && ~/.local/bin/testenv-reap`.
Expected: exit 0; `ls ~/.claude/run-logs/testenv-reap/` shows one run whose last line is a `done` event. Mission Control's Automations page shows a "Test Mac Cleanup" card marked healthy, not "needs registering".

---

## Part B: Meetily's test setup

All in the Meetily worktree `meetily-fork-worktrees/test-environment` (this branch). File map:
- `testenv/golden.pkr.hcl`: Packer template.
- `testenv/provision/audio.sh`, `testenv/provision/tcc.sh`, `testenv/provision/ui.sh`: run inside the guest by Packer.
- `testenv/zoom-standin/main.swift`, `testenv/zoom-standin/Info.plist`, `testenv/zoom-standin/build.sh`.
- `testenv/clips/make-clips.sh`.
- `testenv/ui/ui.mjs`, `testenv/ui/package.json`.
- `testenv/build-test-app.sh`.
- `testenv/README.md`.
- `frontend/src-tauri/Cargo.toml`, `frontend/src-tauri/src/lib.rs`: the `testenv` feature.

### Task 6: `bake` in testenv and Meetily's Packer template

**Files:**
- Create (testenv repo, new branch `feat/bake`): `lib/bake.sh`, `test/stubs/packer`, `test/bake.bats`
- Create (Meetily): `testenv/golden.pkr.hcl`, `testenv/provision/audio.sh`, `testenv/provision/tcc.sh`, `testenv/clips/make-clips.sh`

**Interfaces:**
- Consumes: `recipe_hash`, `hash_file`.
- Produces: `cmd_bake <app> --recipe <dir>`: runs `packer init` and `packer build -force -var vm_name=<app>-golden -var model_dir=<host models/parakeet> <dir>/golden.pkr.hcl` with working dir `<dir>`, then writes `recipe_hash <dir>` to `hash_file <app>`. Exit 2 if Packer fails.
- Packer variable `model_dir` defaults to `~/Library/Application Support/com.meetily.ai/models/parakeet`.

- [ ] **Step 1: Write the failing test (testenv repo)**

`test/stubs/packer`:
```bash
#!/bin/bash
echo "$*" >> "${STUB_STATE:?}/packer.calls"; [ -f "$STUB_STATE/packer_fail" ] && exit 1; exit 0
```

`test/bake.bats`:
```bash
load helper

@test "bake runs packer and records the recipe hash" {
  export TESTENV_PACKER="$BATS_TEST_DIRNAME/stubs/packer" TESTENV_HASH_FILE="$BATS_TEST_TMPDIR/hash"
  mkdir -p "$BATS_TEST_TMPDIR/r" && echo x > "$BATS_TEST_TMPDIR/r/golden.pkr.hcl"
  run "$TESTENV" bake meetily-fork --recipe "$BATS_TEST_TMPDIR/r"
  [ "$status" -eq 0 ]
  grep -q 'build -force -var vm_name=meetily-fork-golden' "$STUB_STATE/packer.calls"
  [ -s "$TESTENV_HASH_FILE" ]
}

@test "bake exits 2 and writes no hash when packer fails" {
  export TESTENV_PACKER="$BATS_TEST_DIRNAME/stubs/packer" TESTENV_HASH_FILE="$BATS_TEST_TMPDIR/hash"
  mkdir -p "$BATS_TEST_TMPDIR/r" && echo x > "$BATS_TEST_TMPDIR/r/golden.pkr.hcl"; touch "$STUB_STATE/packer_fail"
  run "$TESTENV" bake meetily-fork --recipe "$BATS_TEST_TMPDIR/r"
  [ "$status" -eq 2 ]
  [ ! -e "$TESTENV_HASH_FILE" ]
}
```

- [ ] **Step 2: Run to see it fail**

Run: `bats test/bake.bats`
Expected: FAIL, `cmd_bake: command not found`.

- [ ] **Step 3: Write `lib/bake.sh`**

```bash
# Build an app's prepared image with Packer and record which recipe made it.
PACKER="${TESTENV_PACKER:-/opt/homebrew/bin/packer}"
cmd_bake() {
  local app="$1"; shift; local recipe=""
  while [ $# -gt 0 ]; do case "$1" in --recipe) recipe="$2"; shift 2;; *) die_env "unknown option $1";; esac; done
  [ -f "$recipe/golden.pkr.hcl" ] || die_env "no golden.pkr.hcl in $recipe"
  local models="$HOME/Library/Application Support/com.meetily.ai/models/parakeet"
  ( cd "$recipe" && PATH="$(dirname "$TART"):$PATH" "$PACKER" init golden.pkr.hcl >/dev/null &&
    PATH="$(dirname "$TART"):$PATH" "$PACKER" build -force -var "vm_name=$app-golden" -var "model_dir=$models" golden.pkr.hcl ) \
    || die_env "packer build failed for $app-golden"
  mkdir -p "$(dirname "$(hash_file "$app")")"; recipe_hash "$recipe" > "$(hash_file "$app")"
}
```

`recipe_hash` ignores dot-files but not `build/`; the template writes generated files into `testenv/build/`, so `recipe_hash` must skip it. Change its `find` in `lib/vm.sh` to `find . -type f ! -name '.*' ! -path './build/*' -print0`, and add to `test/bake.bats`:
```bash
@test "files under build/ do not change the recipe hash" {
  mkdir -p "$BATS_TEST_TMPDIR/r/build" && echo x > "$BATS_TEST_TMPDIR/r/golden.pkr.hcl"
  h1=$( . "$BATS_TEST_DIRNAME/../lib/common.sh"; . "$BATS_TEST_DIRNAME/../lib/vm.sh"; recipe_hash "$BATS_TEST_TMPDIR/r" )
  echo y > "$BATS_TEST_TMPDIR/r/build/clip.wav"
  h2=$( . "$BATS_TEST_DIRNAME/../lib/common.sh"; . "$BATS_TEST_DIRNAME/../lib/vm.sh"; recipe_hash "$BATS_TEST_TMPDIR/r" )
  [ "$h1" = "$h2" ]
}
```

The tart plugin finds `tart` on PATH, which is why the Tart app folder is put first.

- [ ] **Step 4: Run tests, commit, PR (testenv repo)**

Run: `make test lint` → PASS (18 tests). Then commit "Add bake", `test-gate run -- make test`, push, PR.

- [ ] **Step 5: Write the clips script (Meetily)**

`testenv/clips/make-clips.sh`:
```bash
#!/bin/bash
# Two known sentences for the You and Them checks. Words chosen to be easy to find in a transcript.
set -euo pipefail
out="${1:?output dir}"; mkdir -p "$out"
say -v Samantha -o "$out/you.aiff" "The blue giraffe orders seven pancakes."
say -v Daniel -o "$out/them.aiff" "The orange submarine visits Tuesday harbor."
for c in you them; do afconvert -f WAVE -d LEI16@48000 "$out/$c.aiff" "$out/$c.wav"; rm "$out/$c.aiff"; done
```

- [ ] **Step 6: Write the provision scripts (Meetily)**

`testenv/provision/audio.sh`:
```bash
#!/bin/bash
# Two separate virtual devices, so call audio never leaks into the microphone:
#   BlackHole 2ch  = the fake microphone (default input; `testenv play mic` writes into it with sox)
#   BlackHole 16ch = the fake speakers (default output; `testenv play call` plays there with afplay,
#                    and Meetily's system-audio capture hears it as Them)
# The VM runs with --no-audio, so these are its only audio devices.
set -euo pipefail
eval "$(/opt/homebrew/bin/brew shellenv)"
brew install --cask blackhole-2ch blackhole-16ch
brew install sox switchaudio-osx
SwitchAudioSource -t input -s "BlackHole 2ch"
SwitchAudioSource -t output -s "BlackHole 16ch"
SwitchAudioSource -t system -s "BlackHole 16ch"
SwitchAudioSource -c -t input; SwitchAudioSource -c -t output
```

`testenv/provision/tcc.sh` (row layout copied from GitHub's `actions/runner-images` `configure-tccdb-macos.sh`, columns: service, client, client_type, auth_value, auth_reason, auth_version, csreq, policy_id, indirect_object_identifier_type, indirect_object_identifier, indirect_object_code_identity, flags, last_modified, pid, pid_version, boot_uuid, last_reminded):
```bash
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
```

- [ ] **Step 7: Write the Packer template (Meetily)**

`testenv/golden.pkr.hcl`:
```hcl
# Meetily's prepared test Mac: one layer on Cirrus's sequoia base image,
# in the layered style of cirruslabs/macos-image-templates.
packer {
  required_plugins {
    tart = { version = ">= 1.21.0", source = "github.com/cirruslabs/tart" }
  }
}

variable "vm_name"   { type = string }
variable "model_dir" { type = string }

source "tart-cli" "golden" {
  vm_base_name = "ghcr.io/cirruslabs/macos-sequoia-base:latest"
  vm_name      = var.vm_name
  cpu_count    = 4
  memory_gb    = 8
  headless     = true
  ssh_username = "admin"
  ssh_password = "admin"
  ssh_timeout  = "180s"
  run_extra_args = ["--no-audio"]
}

build {
  sources = ["source.tart-cli.golden"]

  # Files made on the host at build time; `generated = true` skips Packer's up-front existence check.
  provisioner "shell-local" {
    inline = ["bash clips/make-clips.sh build/clips", "bash zoom-standin/build.sh build/zoom-standin"]
  }
  provisioner "shell" {
    inline = ["mkdir -p ~/clips ~/testenv-ui '/Users/admin/Library/Application Support/com.meetily.ai/models'"]
  }
  provisioner "file" {
    source      = "build/clips/"
    destination = "/Users/admin/clips/"
    generated   = true
  }
  provisioner "file" {
    source      = "build/zoom-standin/"
    destination = "/Users/admin/"
    generated   = true
  }
  provisioner "file" {
    source      = var.model_dir
    destination = "/Users/admin/Library/Application Support/com.meetily.ai/models/"
  }
  provisioner "file" {
    source      = "ui/"
    destination = "/Users/admin/testenv-ui/"
  }
  provisioner "file" {
    source      = "launch-app.sh"
    destination = "/Users/admin/launch-app.sh"
  }
  provisioner "shell" {
    inline = ["sudo mv '/Users/admin/Zoom Stand-in.app' /Applications/"]
  }
  provisioner "shell" {
    script = "provision/audio.sh"
  }
  provisioner "shell" {
    script = "provision/tcc.sh"
  }
  provisioner "shell" {
    script = "provision/ui.sh"
  }
}
```

HCL allows only one argument in a one-line block, so every block here is multi-line. Add `testenv/build/` and `testenv/ui/node_modules/` to the repo `.gitignore`. Task 7 writes the stand-in; Task 9 writes `ui/`, `launch-app.sh` and `provision/ui.sh`; Step 8 runs after both. Run `packer validate -var vm_name=x -var model_dir=/tmp testenv/golden.pkr.hcl` (from `testenv/`, after `packer init`) once those files exist, before baking.

- [ ] **Step 8: Bake for real (after Tasks 7 and 9)**

Run: `testenv bake meetily-fork --recipe testenv`
Expected: Packer finishes with `Build 'tart-cli.golden' finished`; `tart list` shows `meetily-fork-golden`; the tcc.sh output lists the three services; the audio.sh output ends with `BlackHole 2ch` then `BlackHole 16ch`. Time it; expect about 15 minutes.

- [ ] **Step 9: Commit (Meetily)**

```bash
git add testenv .gitignore && git commit -m "Add the Packer template for Meetily's test Mac"
```

### Task 7: Zoom stand-in

**Files:**
- Create: `testenv/zoom-standin/main.swift`, `testenv/zoom-standin/Info.plist`, `testenv/zoom-standin/build.sh`

**Interfaces:**
- Produces: `<out>/Zoom Stand-in.app` with bundle id `us.zoom.xos`, one window titled "Zoom Meeting", 900x600, whose slide number and colour change every 4 s. Meetily's `assistant/vision/capture.rs` selects windows by that bundle id and a non-empty title.

- [ ] **Step 1: Write the app**

`testenv/zoom-standin/main.swift`:
```swift
// Test-only stand-in for Zoom: Meetily's capture picks windows by Zoom's bundle id
// (us.zoom.xos, see assistant/vision/capture.rs) and title, so this window is captured
// like a real Zoom meeting. The slide changes every 4 s so the change detector keeps frames.
import AppKit

let app = NSApplication.shared
app.setActivationPolicy(.regular)
let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 900, height: 600),
                      styleMask: [.titled, .closable], backing: .buffered, defer: false)
window.title = "Zoom Meeting"
let label = NSTextField(labelWithString: "")
label.font = .systemFont(ofSize: 72, weight: .bold)
label.frame = NSRect(x: 0, y: 250, width: 900, height: 100)
label.alignment = .center
window.contentView?.addSubview(label)
let colours: [NSColor] = [.systemBlue, .systemOrange, .systemGreen, .systemPurple]
var slide = 0
func show() {
  window.contentView?.wantsLayer = true
  window.contentView?.layer?.backgroundColor = colours[slide % colours.count].cgColor
  label.stringValue = "Slide \(slide + 1)"
  slide += 1
}
show()
Timer.scheduledTimer(withTimeInterval: 4, repeats: true) { _ in show() }
window.makeKeyAndOrderFront(nil)
app.activate(ignoringOtherApps: true)
app.run()
```

`testenv/zoom-standin/Info.plist`:
```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>us.zoom.xos</string>
  <key>CFBundleName</key><string>Zoom Stand-in</string>
  <key>CFBundleExecutable</key><string>zoom-standin</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
</dict></plist>
```

`testenv/zoom-standin/build.sh`:
```bash
#!/bin/bash
# Build the stand-in app bundle on the host (Xcode 26.3).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; out="${1:?output dir}"
app="$out/Zoom Stand-in.app"; rm -rf "$app"; mkdir -p "$app/Contents/MacOS"
swiftc -O -o "$app/Contents/MacOS/zoom-standin" "$here/main.swift"
cp "$here/Info.plist" "$app/Contents/Info.plist"
codesign -s - --force "$app"
```

- [ ] **Step 2: Check it on the host**

Run: `bash testenv/zoom-standin/build.sh /tmp/zs && open "/tmp/zs/Zoom Stand-in.app" && sleep 2 && osascript -e 'tell application "System Events" to get name of every window of (first process whose bundle identifier is "us.zoom.xos")'`
Expected: prints `Zoom Meeting`. Quit it with `osascript -e 'quit app id "us.zoom.xos"'` and `rm -rf /tmp/zs`.

- [ ] **Step 3: Commit**

```bash
git add testenv/zoom-standin && git commit -m "Add a Zoom stand-in window for capture tests"
```

### Task 8: The `testenv` cargo feature and test build script

**Files:**
- Modify: `frontend/src-tauri/Cargo.toml` (`[features]` and `[dependencies]`)
- Modify: `frontend/src-tauri/src/lib.rs:451-472` (plugin chain)
- Create: `testenv/build-test-app.sh`
- Modify: `docs/FORK_CHANGES.md`

**Interfaces:**
- Produces: `testenv/build-test-app.sh` prints the path of the built test `.app` on its last line. That app, started with env `TAURI_WEBDRIVER_PORT=4445`, serves W3C WebDriver on `127.0.0.1:4445` inside the guest.

- [ ] **Step 1: Write the failing check**

Add to `testenv/build-test-app.sh` the guard that later steps rely on, then run it before the feature exists:
```bash
#!/bin/bash
# Build the release app with the WebDriver add-on, for test Macs only.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"; cd "$root/frontend"
export PATH="$HOME/.cargo/bin:$PATH"
conf=$(mktemp -t tauri-testenv).json
jq '.app.security.capabilities |= map(if .identifier == "main" then .permissions += ["wdio-webdriver:default"] else . end)' src-tauri/tauri.conf.json > "$conf"
# Same GPU feature the release build picks (scripts/tauri-auto.js), plus the add-on.
gpu=$(node scripts/auto-detect-gpu.js); features="testenv"; [ -n "$gpu" ] && [ "$gpu" != none ] && features="$gpu,testenv"
pnpm tauri build --config "$conf" -- --features "$features"
# The repo root is a cargo workspace, so bundles land in <root>/target.
app="$root/target/release/bundle/macos/meetily-fork.app"
LC_ALL=C grep -aq 'wdio-webdriver' "$app/Contents/MacOS/meetily" || { echo "built app has no WebDriver add-on" >&2; exit 1; }
otool -L "$app/Contents/MacOS/meetily" | grep -q /opt/homebrew && { echo "built app links Homebrew" >&2; exit 1; }
echo "$app"
```
Run: `bash testenv/build-test-app.sh`
Expected: FAIL with cargo `error: Package ... does not have the feature testenv`.

The `jq` path is checked: `tauri.conf.json` has one inline capability, identifier `main`, with 23 permissions. The add-on check greps the binary for the plugin's name string, because a stripped release binary has no symbols for `nm` to find.

- [ ] **Step 2: Add the feature**

`frontend/src-tauri/Cargo.toml`, under `[features]`:
```toml
# Test Macs only: embedded WebDriver server. Never in a build Joseph installs.
testenv = ["dep:tauri-plugin-wdio-webdriver"]
```
Under the macOS `[target.'cfg(target_os = "macos")'.dependencies]` table:
```toml
tauri-plugin-wdio-webdriver = { version = "1.5", optional = true }
```

`frontend/src-tauri/src/lib.rs`, right after `let mut builder = tauri::Builder::default();` (line 451):
```rust
    // Test Macs only (cargo feature `testenv`): embedded WebDriver for the user-mock agent.
    #[cfg(feature = "testenv")]
    {
        builder = builder.plugin(tauri_plugin_wdio_webdriver::init());
    }
```

- [ ] **Step 3: Build and check**

Run: `bash testenv/build-test-app.sh`
Expected: last line is the `.app` path; no error lines. Then confirm the normal build has no add-on: `cd frontend && pnpm run tauri:build && LC_ALL=C grep -ac 'wdio-webdriver' ../target/release/bundle/macos/meetily-fork.app/Contents/MacOS/meetily` prints `0`. Rebuild the test app afterwards, because both builds write the same bundle path.

- [ ] **Step 4: Run the repo tests**

Run: `test-gate detect`, then `test-gate run -- <that suite>`.
Expected: PASS.

- [ ] **Step 5: Record the fork change and commit**

In `docs/FORK_CHANGES.md` Features table add: `| Test Mac add-on | Builds made by testenv/build-test-app.sh include an embedded WebDriver server so the user-mock agent can use the app in a throwaway test Mac. Builds Joseph installs never include it. | Built (PR #<n>) |`. Under "Upstream files touched" add: `- **Test add-on (PR #<n>):** Cargo.toml (feature testenv), lib.rs (one cfg block).` Fill `<n>` when the PR opens.

```bash
git add -A && git commit -m "Add a test-only WebDriver add-on and test build script"
```

### Task 9: `ui.mjs` and its guest install

**Files:**
- Create: `testenv/ui/ui.mjs`, `testenv/ui/package.json`, `testenv/provision/ui.sh`, `testenv/launch-app.sh`

**Interfaces:**
- Consumes: WebDriver on `127.0.0.1:4445` inside the guest (Task 8).
- Produces, run in the guest as `node ~/testenv-ui/ui.mjs <verb> ...`:
  - `wait "<text>" [seconds=30]`: exit 0 when the text is on screen, exit 1 with `not found after Ns: <text>`.
  - `click "<text>"`: clicks the first visible element whose text matches; exit 1 if none.
  - `type "<placeholder or label>" "<text>"`: types into the matching input.
  - `read`: prints the visible text of the window.
  - Exit 3 when the WebDriver server does not answer (app not running).
- `testenv/launch-app.sh` (copied into the guest by the template): starts the app with the WebDriver port and logs to `/tmp/testenv-app.log`.

- [ ] **Step 1: Write the script**

`testenv/ui/package.json`:
```json
{ "name": "testenv-ui", "private": true, "type": "module", "dependencies": { "webdriverio": "^9" } }
```

`testenv/ui/ui.mjs`:
```js
// User-mock's hands inside a test Mac: talks W3C WebDriver to the app's embedded server.
import { remote } from 'webdriverio';

const [verb, a, b] = process.argv.slice(2);
const xpathText = (t) => `//*[normalize-space(text())=${JSON.stringify(t)} or @aria-label=${JSON.stringify(t)} or @placeholder=${JSON.stringify(t)}]`;

let browser;
try {
  browser = await remote({ hostname: '127.0.0.1', port: 4445, capabilities: {}, logLevel: 'error', connectionRetryCount: 1 });
} catch {
  console.error('app WebDriver not answering on 127.0.0.1:4445 (is the app running?)');
  process.exit(3);
}

async function find(t, secs) {
  const el = await browser.$(xpathText(t));
  await el.waitForDisplayed({ timeout: secs * 1000 }).catch(() => {});
  return (await el.isDisplayed().catch(() => false)) ? el : null;
}

const secs = Number(b ?? 30);
switch (verb) {
  case 'wait': if (!(await find(a, secs))) { console.error(`not found after ${secs}s: ${a}`); process.exit(1); } break;
  case 'click': { const el = await find(a, 10); if (!el) { console.error(`nothing to click: ${a}`); process.exit(1); } await el.click(); break; }
  case 'type': { const el = await find(a, 10); if (!el) { console.error(`no field: ${a}`); process.exit(1); } await el.setValue(b); break; }
  case 'read': console.log(await browser.$('body').getText()); break;
  default: console.error('usage: ui.mjs wait|click|type|read'); process.exit(64);
}
```

`testenv/provision/ui.sh`:
```bash
#!/bin/bash
set -euo pipefail
eval "$(/opt/homebrew/bin/brew shellenv)"
cd ~/testenv-ui && npm install --omit=dev
```

`testenv/launch-app.sh`:
```bash
#!/bin/bash
# Start Meetily in a test Mac with the WebDriver port; open --env keeps the app its own responsible process for permissions.
open --env TAURI_WEBDRIVER_PORT=4445 --stdout /tmp/testenv-app.log --stderr /tmp/testenv-app.log -a /Applications/meetily-fork.app
```
The Task 6 template already copies `launch-app.sh` into the guest.

- [ ] **Step 2: Check `ui.mjs` on the host against a real test build**

Run on the host, from a copy so no `node_modules` lands in the recipe folder (it would change the recipe hash): `rm -rf /tmp/ui-check && cp -R testenv/ui /tmp/ui-check && (cd /tmp/ui-check && npm install) && TAURI_WEBDRIVER_PORT=4445 "<path from build-test-app.sh>/Contents/MacOS/meetily" & sleep 15 && node /tmp/ui-check/ui.mjs read | head -5 && node /tmp/ui-check/ui.mjs wait "No such text anywhere" 3; echo "exit=$?"`
Expected: the first lines of the app's window text; then `not found after 3s: No such text anywhere` and `exit=1`. Stop the app by pid.

- [ ] **Step 3: Check the no-app case**

Run: `node /tmp/ui-check/ui.mjs read; echo "exit=$?"` with no app running, then `rm -rf /tmp/ui-check`.
Expected: `app WebDriver not answering ...` and `exit=3`.

- [ ] **Step 4: Commit**

```bash
git add testenv && git commit -m "Add ui.mjs for driving the app in a test Mac"
```

Now run Task 6 Step 8 (bake).

---

## Part C: agents

`~/.claude` is a local git repo; commit there directly.

### Task 10: `testenv/README.md` and the user-mock agent

**Files:**
- Create: `testenv/README.md` (Meetily), `~/.claude/agents/user-mock.md`

**Interfaces:**
- Consumes: every testenv command; `ui.mjs` verbs; `~/launch-app.sh`; clips at `/Users/admin/clips/{you,them}.wav`.
- Produces: the user-mock report shape that Task 11 parses:
  ```
  RESULT: PASS | FAIL | ENVIRONMENT
  | # | Step | Result | Saw | Screenshot |
  ...
  Other problems: <lines or "none">
  ```

- [ ] **Step 1: Write `testenv/README.md` (for user-mock; no code in it)**

Contents, in this order:
1. Build: `bash testenv/build-test-app.sh` (its last line is the app path). User-mock receives that path; it does not build.
2. Start: `id=$(testenv up meetily-fork --app <app path> --recipe <repo>/testenv)`, then `testenv exec $id -- bash ~/launch-app.sh`.
3. Drive: `testenv exec $id -- node ~/testenv-ui/ui.mjs wait|click|type|read ...`; screenshot after every step with `testenv shot $id <step-name>`.
4. Audio: `testenv play $id mic /Users/admin/clips/you.wav` (expected words: "blue giraffe", "seven pancakes", labelled You); `testenv play $id call /Users/admin/clips/them.wav` (expected words: "orange submarine", "Tuesday harbor", labelled Them).
5. Zoom: `testenv exec $id -- open -a "Zoom Stand-in"`; during a recording, `testenv exec $id -- bash -c 'ls ~/Movies/meetily-recordings/*/zoom-frames/ 2>/dev/null | head'` must list picture files. The app saves frames in `zoom-frames` inside the meeting folder (default `~/Movies/meetily-recordings/<meeting>/`), see `zoom_frames_dir` in `frontend/src-tauri/src/assistant/core.rs`.
6. The three standing checks from the spec, word for word.
7. Finish: `testenv collect $id`, then `testenv down $id`, always, even after a failure.
8. Exit code 2 from any testenv command is an environment failure.


- [ ] **Step 2: Write the agent**

`~/.claude/agents/user-mock.md`:
```markdown
---
name: user-mock
description: "Uses an app the way Joseph would, in a fresh throwaway test environment, and reports pass or fail per step with screenshots. Started by dev-lead before every PR; Joseph can also ask for a walk-through directly. Never reads code, never fixes anything."
model: sonnet
tools: Bash, Read
---

You are Joseph's stand-in user. You check that an app works for a person, not that its code is right.

## What you get
Joseph's request, the "done when" checks, the built app's path, and the repo path.

## What you read
Only the repo's `CLAUDE.md` Goal and Stories sections and `testenv/README.md` (Mac apps) or the repo's run recipe and test database recipe (web apps). Never source files, diffs, tests or commit history: you judge the app the way Joseph would.

## How you work
1. Start a fresh environment exactly as the README says. Mac apps: `testenv up`. Web apps, until a lab environment exists: the repo's run recipe against its test database, driven with Playwright.
2. Walk the "done when" checks, then every user story you can reach without Claude, then the README's standing checks.
3. Screenshot every step and `Read` each shot before you call the step passed.
4. A permission prompt, crash dialog or blank window is a failed step, never something to work around.
5. Always collect logs and tear the environment down, even after a failure.
6. Any testenv command that exits 2 is an environment failure: stop, tear down, report RESULT: ENVIRONMENT.

## Rules
- Never edit files, commit, open a PR, or start a third test Mac.
- Never copy a Claude login, token or `.env` file into a test environment.
- Plain words in the report: name the screen and the button, as Joseph would see them.

## Report
RESULT: PASS | FAIL | ENVIRONMENT
| # | Step | Result | Saw | Screenshot |
Other problems: anything that looked broken outside the steps, or "none".
```

- [ ] **Step 3: Commit**

```bash
git -C ~/.claude add agents/user-mock.md && git -C ~/.claude commit -m "Add user-mock agent"
git add testenv/README.md && git commit -m "Add the test Mac guide for user-mock"
```

### Task 11: dev-lead and CLAUDE.md

**Files:**
- Modify: `~/.claude/agents/dev-lead.md` section 7 step 2 and section 9 report list
- Modify: `~/.claude/CLAUDE.md` "Code goes to dev-lead" section

- [ ] **Step 1: Replace dev-lead section 7 step 2**

Old text (exact):
```
2. Start the app with the repo's run recipe against its test database and walk every slice's "done when"
   check. Screenshot every screen at desktop and phone size and `Read` the shots back.
```
New text:
```
2. Start one `user-mock` agent (no `name`) with Joseph's request, every slice's "done when" check, the
   built app's path (Mac apps: the last line of `testenv/build-test-app.sh`) and the repo path. You do not
   start the app or take screenshots yourself. RESULT: FAIL sends each failed step, what user-mock saw and
   its screenshot to that slice's developer; after the fix, start a fresh user-mock on the new build.
   RESULT: ENVIRONMENT goes to Joseph, never to a developer.
```
Also in section 8, after "the test steps", add: "and user-mock's step table".

- [ ] **Step 2: Add the CLAUDE.md line**

At the end of the "Code goes to dev-lead" paragraph in `~/.claude/CLAUDE.md`, add:
```
Dev-lead hands the walk-through of the running app to `user-mock`, which uses it in a fresh test environment and never reads the code; Joseph can also ask for a user-mock walk-through directly.
```

- [ ] **Step 3: Commit**

```bash
git -C ~/.claude add agents/dev-lead.md CLAUDE.md && git -C ~/.claude commit -m "dev-lead hands the app walk-through to user-mock"
```

### Task 12: Prove it end to end, then PR

**Files:**
- Modify: `CLAUDE.md` (Meetily): add a line under "Testing and Debugging": "Before a PR, user-mock walks the app in a throwaway test Mac: `testenv/README.md`."

- [ ] **Step 1: Build the test app**

Run: `bash testenv/build-test-app.sh`
Expected: last line is the app path.

- [ ] **Step 2: Start user-mock on it**

From this session, spawn `user-mock` (foreground) with: request "prove the test environment on current main", done-when "the three standing checks in testenv/README.md pass", the app path, and this worktree's path.
Expected: `RESULT: PASS`, three rows, each with a screenshot path under `~/.cache/testenv/runs/`. `Read` the three screenshots yourself and confirm they show what the rows say. Confirm `testenv ls` lists nothing afterwards.

- [ ] **Step 3: Check the crash path once by hand (Review Focus 4)**

Run: `id=$(testenv up meetily-fork --app <app> --recipe testenv) && testenv exec $id -- /bin/bash -c 'mv /Applications/meetily-fork.app/Contents/MacOS/meetily /tmp/m && bash ~/launch-app.sh'; testenv exec $id -- node ~/testenv-ui/ui.mjs wait "Welcome" 20; echo "exit=$?"; testenv collect $id; testenv down $id`
Expected: `exit=3` or `exit=1` with a clear message, `collect` prints a folder, `testenv ls` is empty after `down`.

- [ ] **Step 4: Check the cleanup job (spec Part 5 step 2)**

Run: `id=$(testenv up meetily-fork --app <app> --recipe testenv) && echo $(( $(date +%s) - 7300 )) > ~/.cache/testenv/runs/$id/created && ~/.local/bin/testenv-reap && testenv ls`
Expected: `testenv ls` prints nothing; Mission Control shows the `testenv-reap` run as done.

- [ ] **Step 5: Commit, gate, PR**

```bash
git add -A && git commit -m "Point Meetily's testing notes at the test Mac guide"
test-gate run -- <suite from test-gate detect>
git push -u origin docs/test-environment
gh pr create --base main --title "Test Macs for Meetily: Packer image, test add-on, user-mock guide" --body "Spec and plan in docs/superpowers. User-mock passed all three standing checks on a fresh test Mac (screenshots in the run folder). Also updates the Xcode note in CLAUDE.md."
```
Fill the PR number into the two `docs/FORK_CHANGES.md` lines from Task 8, commit, re-run the gate, push.

- [ ] **Step 6: Hand Joseph the result**

Report: the two PR links (testenv, Meetily) plus the My-Life router PR, the three screenshots, how long `bake` and one user-mock run took, and that `testenv ls` is empty.
