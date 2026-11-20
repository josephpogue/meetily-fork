---
paths:
  - "frontend/src-tauri/**"
  - "frontend/src/**"
---

# Development patterns

## 1. Audio Buffer Management

**Ring Buffer Mixing** (pipeline.rs):
- Mic and system audio arrive asynchronously at different rates
- Ring buffer accumulates samples until both streams have aligned windows (50ms)
- Professional mixing applies RMS-based ducking to prevent system audio from drowning out microphone
- Uses `VecDeque` for efficient windowed processing

## 2. Thread Safety and Async Boundaries

**Recording State** (recording_state.rs):
```rust
pub struct RecordingState {
    is_recording: Arc<AtomicBool>,
    audio_sender: Arc<RwLock<Option<mpsc::UnboundedSender<AudioChunk>>>>,
    // ...
}
```

**Key Pattern**: Use `Arc<RwLock<T>>` for shared state across async tasks, `Arc<AtomicBool>` for simple flags.

## 3. Error Handling and Logging

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

## 4. Frontend State Management

**Sidebar Context** (components/Sidebar/SidebarProvider.tsx):
- Global state for meetings list, current meeting, recording status
- Communicates with the Rust/Tauri core through Tauri commands and events
- Keeps React state synchronized with native recording, meeting, transcript, and summary state

**Pattern**: Tauri commands update Rust state → Emit events → Frontend listeners update React state → Context propagates to components
