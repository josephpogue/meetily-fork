# Meetily fork: platform notes

### macOS
- **Audio Capture**: Uses ScreenCaptureKit for system audio (macOS 13+)
- **GPU**: Metal + CoreML automatically enabled
- **Permissions**: Requires microphone + screen recording permissions
- **System Audio**: Requires virtual audio device (BlackHole) for system capture
- **Worktree builds**: Full Xcode 26.3 is installed and selected (since 2026-10-14), so the `cidre` build script's `xcodebuild` call works and a fresh worktree builds normally with `PATH=$HOME/.cargo/bin:$PATH`. Copy `<main>/frontend/src-tauri/binaries/*` into the worktree first (gitignored sidecars the tauri build requires). `scripts/xcodebuild-shim/xcodebuild` is a fallback for a Mac with only Command Line Tools: put it first on PATH and it compiles cidre's pomace targets with clang.
- **Signing and installing (this Mac)**: ad hoc signing (`signingIdentity: "-"`, still the default in `tauri.conf.json` so CI and other Macs build) gives each build a new code hash, and macOS pins the Screen Recording grant to it, so after a rebuild the toggle looks on but capture silently fails. Run `scripts/setup-mac-signing.sh` once (creates the self-signed "meetily-fork local" identity in the login keychain, idempotent), then build and install with `scripts/install-mac.sh --build`, which signs with that identity through `APPLE_SIGNING_IDENTITY`, copies the app to /Applications, and when the designated requirement changed runs `tccutil reset ScreenCapture com.meetily.ai`, opens the app and tells you to click Allow. Check a build with `codesign -d -r- <app>`: it must show `certificate leaf = H"..."`, never `cdhash`. Quit the app before installing; the script refuses while it runs. A grant that still fails: `scripts/install-mac.sh --reset-tcc`.
- **Test Mac screen capture**: an app that captures the screen needs its bundle id pre-approved in replayd's `ScreenCaptureApprovals.plist` by `testenv/provision/tcc.sh`, or Sequoia covers the screen with "<app> is requesting to bypass the system private window picker". Meetily's entry is `com.meetily.ai`.
- **Test Mac notifications**: macOS 15 keeps an app's notification grant in `~/Library/Preferences/com.apple.ncprefs.plist`; `testenv/provision/tcc.sh` pre-sets Meetily's. usernoted drops an entry at boot whose `path` does not exist, so that entry's path must name an app present at boot.
- **Frontend tests** are `bun test` in `frontend/`. bun is not installed globally on this Mac: `npm i bun --prefix /tmp/<dir>` and run its `node_modules/.bin/bun`.
- **No Homebrew libraries in the app**: `frontend/src-tauri/.cargo/config.toml` sets `LIBONNXRUNTIME_NO_PKG_CONFIG`, so onnxruntime links statically even on a Mac with Homebrew's onnxruntime. It applies only to cargo run from `frontend/src-tauri` (tauri does that), not to `cargo test` from the repo root. Check a built app with `otool -L <app>/Contents/MacOS/meetily | grep /opt/homebrew`, which must print nothing.

### Windows
- **Audio Capture**: Uses WASAPI (Windows Audio Session API)
- **GPU**: CUDA (NVIDIA) or Vulkan (AMD/Intel) via Cargo features
- **Build Tools**: Requires Visual Studio Build Tools with C++ workload
- **System Audio**: Uses WASAPI loopback for system capture

### Linux
- **Audio Capture**: ALSA/PulseAudio
- **GPU**: CUDA (NVIDIA) or Vulkan via Cargo features
- **Dependencies**: Requires cmake, llvm, libomp
