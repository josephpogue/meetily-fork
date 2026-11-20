# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Goals

@docs/GOAL.md

Read docs/GOAL.md before planning any change. Section goals live in docs/goals/; each lists the code folders it governs in its front matter. Before changing code, read every goal file whose `code` covers the files you touch, plus each of its parents.

- When a change would cross a Non-goal or a Constraint, stop and ask Joseph.
- When a change alters what a section should do, edit that goal file in the same pull request, with Joseph's approval.

## Fork changes and upstream merges

- Upstream is the `upstream` remote (Zackriya-Solutions/meetily). Merge a new release with `git fetch upstream` and a merge branch, as in PR #3.
- Any PR that adds or changes fork behaviour also updates `docs/FORK_CHANGES.md`: what it does, the PR number, and the upstream files it touches. A new decision Joseph makes about this version goes under Requirements in `docs/GOAL.md`.
- After an upstream merge, update the upstream base version there and check that each listed change still works.

## Read on demand

| Before you... | Open |
|---|---|
| need what the app is, its technology stack, the Tauri desktop diagram or the audio pipeline (recording path versus transcription path) | `docs/project-overview.md` |
| edit Rust or frontend code (buffer mixing, thread safety, `perf_debug!` logging, Sidebar state) or tune audio, Whisper or frontend performance | `.claude/rules/development-patterns.md` and `.claude/rules/performance.md` (load when you open a file under `frontend/src-tauri/` or `frontend/src/`) |
| debug with Rust logs, DevTools or the audio pipeline metrics | `docs/debugging.md` |
| build in a worktree, sign or install the app, fix a Screen Recording grant that fails, set up the test Mac, run frontend tests or check Homebrew linking (macOS), or work on Windows or Linux | `docs/platform-notes.md` |

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

### Audio Device Modularization (Recently Completed)

The `audio/` module tree and the map from symptom to owning file live in `docs/architecture.md`. **Read `docs/architecture.md` before changing anything under `frontend/src-tauri/src/audio/`.**

### Rust <-> Frontend Communication, and Whisper Model Management

The Tauri command and event patterns, the Whisper model storage paths per platform, and the GPU feature flags live in `docs/architecture.md`. **Read `docs/architecture.md` before adding a Tauri command or event, and before touching model loading.**

## Common Development Tasks

Recipes for adding an audio device platform, adding a Tauri command, and modifying the audio pipeline live in `docs/common-tasks.md`. **Read `docs/common-tasks.md` before starting any of those three.**

### Tauri Backend Development

Current app behavior should be implemented in the Rust/Tauri core, not in the archived Python backend. Add new frontend-facing behavior through Tauri commands/events and existing Rust services under `frontend/src-tauri/src`.

Do not add new endpoints to `backend/app/main.py`; that FastAPI code is legacy archive material only.

## Testing and Debugging

Before a PR, user-mock walks the app in a throwaway test Mac: `testenv/README.md`.

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
