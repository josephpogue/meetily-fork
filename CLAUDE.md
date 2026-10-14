# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Goal

Joseph's own build of Meetily that helps him run a call while it happens, mainly as a consultant on discovery and knowledge-transfer calls.
- Follow the live transcript and the point of the call, so a suggested answer is ready when someone asks him a question.
- Keep a live list of the gaps it sees, given the point of the call, with a question he can ask to fill each one.
- Take the point of the call from an optional meeting brief he can edit during the call, and from the conversation itself.
- Stay close to upstream Meetily, so every upstream release merges in cheaply, and add only what his use needs.

Every difference from upstream Meetily, built or planned, is listed in `docs/FORK_CHANGES.md`.

### Stories

Each story is one person and one result. Check a change against them: Joseph must still get each result.
- **Answer on the spot:** Someone on the call asks Joseph a question, and a suggested answer is on screen before he has to reply.
- **Fill the gaps:** On a discovery or knowledge-transfer call, Joseph sees what is still unknown and a question to ask for each gap.
- **Gaps clear themselves:** Once the call answers a gap question, it leaves the list without Joseph touching it.
- **Point of the call:** Joseph types a brief before or during the call, or none at all, and the assistant still knows what the call is for.
- **Ask privately:** Joseph types or speaks a question mid-call and gets an answer that uses the call so far.
- **Catch up:** After losing the thread, Joseph presses one key and gets what was just said and what it means.
- **Keep a record:** After the call, a note with decisions, action items and open questions goes into the vault only when Joseph saves it.
- **Stay current:** A new Meetily release merges into the fork, and every feature in `docs/FORK_CHANGES.md` still works.

### Requirements

Decisions Joseph has made for this version. Every feature follows them.
- Every model call is the local `claude` CLI on his subscription. No API keys, no paid APIs, and Meetily's own summary providers stay untouched.
- Audio and transcripts stay on the Mac. The only outside path is Anthropic through the `claude` CLI, and that path also carries pictures of the Zoom meeting window when "See the Zoom window" is on.
- The assistant sees only the Zoom meeting window, and only while recording. It keeps a picture only when the window changes, by the /watch rule in `~/.claude/skills/watch/scripts/frames.py` (scene score above 0.20, then a 16x16 thumbnail more than 2.0 from the last kept picture). Kept pictures are saved full-size in the meeting folder.
- The assistant informs and converses, and never acts. Nothing is written anywhere without a Save press.
- Output is text only, with no spoken replies.
- The app runs on macOS on Apple Silicon.
- The app is a normal window. It is not hidden from screen share, and is never described as hidden.
- The meeting brief is optional and editable during the call. The assistant also works out the point of the call from the conversation.
- Gaps are the assistant's own judgment from the context it has. There is no preset question list or checklist.
- The assistant removes a gap question once the call answers it. Joseph does not tick questions off.
- New code lives in its own modules, and edits to upstream files stay small and are listed in `docs/FORK_CHANGES.md`.
- Upstream releases arrive only by merging into the fork. Meetily's in-app updater stays off, because it would install plain Meetily over the fork.

## Fork changes and upstream merges

- Upstream is the `upstream` remote (Zackriya-Solutions/meetily). Merge a new release with `git fetch upstream` and a merge branch, as in PR #3.
- Any PR that adds or changes fork behaviour also updates `docs/FORK_CHANGES.md`: what it does, the PR number, and the upstream files it touches. A new decision Joseph makes about this version goes under Requirements above.
- After an upstream merge, update the upstream base version there and check that each listed change still works.

## Project Overview

**Meetily** is a privacy-first AI meeting assistant that captures, transcribes, and summarizes meetings entirely on local infrastructure. The supported application is the Tauri desktop app with a Rust core.

1. **Frontend**: Tauri-based desktop application (Rust + Next.js + TypeScript)
2. **Rust Backend**: Tauri commands, audio capture, transcription, storage, and summarization orchestration
3. **Legacy Backend Archive**: the old Python/FastAPI, Docker, and standalone whisper-server backend under `backend/` is archived and unsupported

### Key Technology Stack
- **Desktop App**: Tauri 2.x (Rust) + Next.js 14 + React 18
- **Audio Processing**: Rust (cpal, whisper-rs, professional audio mixing)
- **Transcription**: Whisper.cpp / whisper-rs and Parakeet paths in the Tauri app
- **App API Surface**: Tauri commands and events, not a separate FastAPI service
- **LLM Integration**: Ollama (local), Claude, Groq, OpenRouter

## Essential Development Commands

### Frontend Development (Tauri Desktop App)

**Location**: `/frontend`

```bash
# macOS Development
./clean_run.sh              # Clean build and run with info logging
./clean_run.sh debug        # Run with debug logging
./clean_build.sh            # Production build

# Windows Development
clean_run_windows.bat       # Clean build and run
clean_build_windows.bat     # Production build

# Manual Commands
pnpm install                # Install dependencies
pnpm run dev                # Next.js dev server (port 3118)
pnpm run tauri:dev          # Full Tauri development mode
pnpm run tauri:build        # Production build

# GPU-Specific Builds (for testing acceleration)
pnpm run tauri:dev:metal    # macOS Metal GPU
pnpm run tauri:dev:cuda     # NVIDIA CUDA
pnpm run tauri:dev:vulkan   # AMD/Intel Vulkan
pnpm run tauri:dev:cpu      # CPU-only (no GPU)
```

### Legacy Backend Archive

**Location**: `/backend`

The Python/FastAPI backend, Docker setup, and standalone whisper-server scripts are archived for historical reference and migration context only. Do not use them for current development, new installs, production deployments, or issue triage for the supported app.

The archived FastAPI service had unauthenticated, development-oriented CORS behavior. Treat that behavior as obsolete legacy context, not as a supported production API.

### Service Endpoints
- **Frontend Dev**: http://localhost:3118

## High-Level Architecture

### Tauri Desktop Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    Frontend (Tauri Desktop App)                  │
│  ┌──────────────────┐  ┌─────────────────┐  ┌────────────────┐ │
│  │   Next.js UI     │  │  Rust Backend   │  │ Whisper Engine │ │
│  │  (React/TS)      │←→│  (Audio + IPC)  │←→│  (Local STT)   │ │
│  └──────────────────┘  └─────────────────┘  └────────────────┘ │
│         ↑ Tauri Events           ↑ Audio Pipeline               │
└─────────────────────────────────────────────────────────────────┘
```

The current app does not require a separate FastAPI tier. Meeting persistence, local transcription, and summary orchestration are handled through the Rust/Tauri core.

### Audio Processing Pipeline (Critical Understanding)

The audio system has **two parallel paths** with different purposes:

```
Raw Audio (Mic + System)
         ↓
┌────────────────────────────────────────────────────────────┐
│              Audio Pipeline Manager                         │
│  (frontend/src-tauri/src/audio/pipeline.rs)                │
└─────────────┬──────────────────────────┬───────────────────┘
              ↓                          ↓
    ┌─────────────────┐        ┌─────────────────────┐
    │ Recording Path  │        │ Transcription Path  │
    │ (Pre-mixed)     │        │ (VAD-filtered)      │
    └─────────────────┘        └─────────────────────┘
              ↓                          ↓
    RecordingSaver.save()      WhisperEngine.transcribe()
```

**Key Insight**: The pipeline performs **professional audio mixing** (RMS-based ducking, clipping prevention) for recording, while simultaneously applying **Voice Activity Detection (VAD)** to send only speech segments to Whisper for transcription.

### Audio Device Modularization (Recently Completed)

The `audio/` module tree and the map from symptom to owning file live in `docs/architecture.md`. **Read `docs/architecture.md` before changing anything under `frontend/src-tauri/src/audio/`.**

### Rust <-> Frontend Communication, and Whisper Model Management

The Tauri command and event patterns, the Whisper model storage paths per platform, and the GPU feature flags live in `docs/architecture.md`. **Read `docs/architecture.md` before adding a Tauri command or event, and before touching model loading.**

## Critical Development Patterns

### 1. Audio Buffer Management

**Ring Buffer Mixing** (pipeline.rs):
- Mic and system audio arrive asynchronously at different rates
- Ring buffer accumulates samples until both streams have aligned windows (50ms)
- Professional mixing applies RMS-based ducking to prevent system audio from drowning out microphone
- Uses `VecDeque` for efficient windowed processing

### 2. Thread Safety and Async Boundaries

**Recording State** (recording_state.rs):
```rust
pub struct RecordingState {
    is_recording: Arc<AtomicBool>,
    audio_sender: Arc<RwLock<Option<mpsc::UnboundedSender<AudioChunk>>>>,
    // ...
}
```

**Key Pattern**: Use `Arc<RwLock<T>>` for shared state across async tasks, `Arc<AtomicBool>` for simple flags.

### 3. Error Handling and Logging

**Performance-Aware Logging** (lib.rs):
```rust
#[cfg(debug_assertions)]
macro_rules! perf_debug {
    ($($arg:tt)*) => { log::debug!($($arg)*) };
}

#[cfg(not(debug_assertions))]
macro_rules! perf_debug {
    ($($arg:tt)*) => {};  // Zero overhead in release builds
}
```

**Usage**: Use `perf_debug!()` and `perf_trace!()` for hot-path logging that should be eliminated in production.

### 4. Frontend State Management

**Sidebar Context** (components/Sidebar/SidebarProvider.tsx):
- Global state for meetings list, current meeting, recording status
- Communicates with the Rust/Tauri core through Tauri commands and events
- Keeps React state synchronized with native recording, meeting, transcript, and summary state

**Pattern**: Tauri commands update Rust state → Emit events → Frontend listeners update React state → Context propagates to components

## Common Development Tasks

Recipes for adding an audio device platform, adding a Tauri command, and modifying the audio pipeline live in `docs/common-tasks.md`. **Read `docs/common-tasks.md` before starting any of those three.**

### Tauri Backend Development

Current app behavior should be implemented in the Rust/Tauri core, not in the archived Python backend. Add new frontend-facing behavior through Tauri commands/events and existing Rust services under `frontend/src-tauri/src`.

Do not add new endpoints to `backend/app/main.py`; that FastAPI code is legacy archive material only.

## Testing and Debugging

### Frontend Debugging

**Enable Rust Logging**:
```bash
# macOS
RUST_LOG=debug ./clean_run.sh

# Windows (PowerShell)
$env:RUST_LOG="debug"; ./clean_run_windows.bat
```

**Developer Tools**:
- Open DevTools: `Cmd+Shift+I` (macOS) or `Ctrl+Shift+I` (Windows)
- Console Toggle: Built into app UI (console icon)
- View Rust logs: Check terminal output

### Audio Pipeline Debugging

**Key Metrics** (emitted by pipeline):
- Buffer sizes (mic/system)
- Mixing window count
- VAD detection rate
- Dropped chunk warnings

**Monitor via Developer Console**: The app includes real-time metrics display when recording.

## Platform-Specific Notes

### macOS
- **Audio Capture**: Uses ScreenCaptureKit for system audio (macOS 13+)
- **GPU**: Metal + CoreML automatically enabled
- **Permissions**: Requires microphone + screen recording permissions
- **System Audio**: Requires virtual audio device (BlackHole) for system capture
- **Worktree builds**: Full Xcode 26.3 is installed and selected (since 2026-10-14), so the `cidre` build script's `xcodebuild` call works and a fresh worktree builds normally with `PATH=$HOME/.cargo/bin:$PATH`. Copy `<main>/frontend/src-tauri/binaries/*` into the worktree first (gitignored sidecars the tauri build requires). `scripts/xcodebuild-shim/xcodebuild` is a fallback for a Mac with only Command Line Tools: put it first on PATH and it compiles cidre's pomace targets with clang.
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

## Performance Optimization Guidelines

### Audio Processing
- Use `perf_debug!()` / `perf_trace!()` for hot-path logging (zero cost in release)
- Batch audio metrics using `AudioMetricsBatcher` (pipeline.rs)
- Pre-allocate buffers with `AudioBufferPool` (buffer_pool.rs)
- VAD filtering reduces Whisper load by ~70% (only processes speech)

### Whisper Transcription
- **Model Selection**: Balance accuracy vs speed
  - Development: `base` or `small` (fast iteration)
  - Production: `medium` or `large-v3` (best quality)
- **GPU Acceleration**: 5-10x faster than CPU
- **Parallel Processing**: Available in `whisper_engine/parallel_processor.rs` for batch workloads

### Frontend Performance
- React state updates batched via Sidebar context
- Transcript rendering virtualized for large meetings
- Audio level monitoring throttled to 60fps

## Important Constraints and Gotchas

1. **Audio Chunk Size**: Pipeline expects consistent 48kHz sample rate. Resampling happens at capture time.

2. **Platform Audio Quirks**:
   - macOS: ScreenCaptureKit requires macOS 13+, needs screen recording permission
   - Windows: WASAPI exclusive mode can conflict with other apps
   - System audio requires virtual device (BlackHole on macOS, WASAPI loopback on Windows)

3. **Whisper Model Loading**: Models are loaded once and cached. Changing models requires app restart or manual unload/reload.

4. **No Separate Backend Dependency**: Meeting persistence, transcription, and LLM features are handled by the Tauri app. Do not reintroduce the archived FastAPI backend as a supported requirement.

5. **Legacy FastAPI Security Context**: The archived FastAPI/CORS behavior is unsupported legacy code and must not be treated as a supported production API.

6. **File Paths**: Use Tauri's path APIs (`downloadDir`, etc.) for cross-platform compatibility. Never hardcode paths.

7. **Audio Permissions**: Request permissions early. macOS requires both microphone AND screen recording for system audio.

## Repository-Specific Conventions

- **Logging Format**: Rust logs should include enough module context to diagnose app behavior
- **Error Handling**: Rust uses `anyhow::Result`, frontend uses try-catch with user-friendly messages
- **Naming**: Audio devices use "microphone" and "system" consistently (not "input"/"output")
- **Git Branches**:
  - `main`: Stable releases
  - `fix/*`: Bug fixes
  - `enhance/*`: Feature enhancements

## Key Files Reference

The per-area entry points (core coordination, audio system, UI components, Whisper integration) live in `docs/architecture.md`. **Read `docs/architecture.md` when you need the entry point for an area you have not touched before.**
