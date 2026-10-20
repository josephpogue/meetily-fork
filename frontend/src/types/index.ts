export interface Message {
  id: string;
  content: string;
  timestamp: string;
}

export interface Transcript {
  id: string;
  text: string;
  timestamp: string; // Wall-clock time (e.g., "14:30:05")
  source?: string; // "mic" | "system" | "mixed" (dominant channel, not diarization)
  sequence_id?: number;
  chunk_start_time?: number; // Legacy field
  is_partial?: boolean;
  confidence?: number;
  // NEW: Recording-relative timestamps for playback sync
  audio_start_time?: number; // Seconds from recording start (e.g., 125.3)
  audio_end_time?: number;   // Seconds from recording start (e.g., 128.6)
  duration?: number;          // Segment duration in seconds (e.g., 3.3)
}

export interface TranscriptUpdate {
  text: string;
  timestamp: string; // Wall-clock time for reference
  source: string; // "mic" | "system" | "mixed" (dominant channel, not diarization)
  sequence_id: number;
  chunk_start_time: number; // Legacy field
  is_partial: boolean;
  confidence: number;
  // NEW: Recording-relative timestamps for playback sync
  audio_start_time: number; // Seconds from recording start
  audio_end_time: number;   // Seconds from recording start
  duration: number;          // Segment duration in seconds
}

export interface Block {
  id: string;
  type: string;
  content: string;
  color: string;
}

export interface Section {
  title: string;
  blocks: Block[];
}

export interface Summary {
  [key: string]: Section;
}

export interface ApiResponse {
  message: string;
  num_chunks: number;
  data: any[];
}

export interface SummaryResponse {
  status: string;
  summary: Summary;
  raw_summary?: string;
  usage?: {
    prompt_tokens: number;
    completion_tokens: number;
    total_tokens: number;
  };
}

// BlockNote-specific types
export type SummaryFormat = 'legacy' | 'markdown' | 'blocknote';

export interface BlockNoteBlock {
  id: string;
  type: string;
  props?: Record<string, any>;
  content?: any[];
  children?: BlockNoteBlock[];
}

export interface SummaryDataResponse {
  markdown?: string;
  summary_json?: BlockNoteBlock[];
  reasoning_stripped?: boolean;
  normalization_fallback?: boolean;
  // Legacy format fields
  MeetingName?: string;
  _section_order?: string[];
  [key: string]: any; // For legacy section data
}

export type MeetingSummary = Summary | SummaryDataResponse;

export type SummaryProcessStatus =
  | 'pending'
  | 'processing'
  | 'completed'
  | 'failed'
  | 'cancelled'
  | 'error'
  | 'idle';

export interface ProcessTranscriptResponse {
  message: string;
  process_id: string;
}

export interface CancelSummaryResponse {
  cancelled: boolean;
  message: string;
  meeting_id: string;
}

export interface SummaryProcessResponse {
  status: SummaryProcessStatus;
  meetingName: string | null;
  meeting_id: string;
  start: string | null;
  end: string | null;
  data: unknown | null;
  error: string | null;
}

// Pagination types for optimized transcript loading
export interface MeetingMetadata {
  id: string;
  title: string;
  created_at: string;
  updated_at: string;
  folder_path?: string;
}

export interface PaginatedTranscriptsResponse {
  transcripts: Transcript[];
  total_count: number;
  has_more: boolean;
}

// Transcript segment data for virtualized display
export interface TranscriptSegmentData {
  id: string;
  timestamp: number; // audio_start_time in seconds
  endTime?: number; // audio_end_time in seconds
  text: string;
  confidence?: number;
  source?: string; // "mic" | "system" | "mixed" (dominant channel, not diarization)
}

// Live assistant types (contract pinned in docs/superpowers/specs/2026-09-01-live-assistant-design.md)

export type AssistantCardPhase = 'drafting' | 'checked' | 'corrected';
export type AssistantCardKind = 'answer' | 'ask' | 'explain' | 'catchup';

export interface AssistantCard {
  id: string;
  kind: AssistantCardKind;
  question: string;
  lead: string;
  bullets: string[];
  source: string;
  phase: AssistantCardPhase;
  changedLines: string[];
  ts: number;
}

export interface AssistantStatus {
  enabled: boolean;
  sessionOpen: boolean;
  lanesReady: boolean;
  mode: 'manual' | 'gated' | 'continuous';
  listening: boolean;
  claudeOk: boolean;
  lastError: string | null;
  // Bumped by the backend only when last_error is set to a new value, never
  // on unrelated status changes (mode/listening/enabled toggles). Lets the
  // panel tell "a fresh error just arrived" apart from "the same error text
  // is still sitting in state from before".
  lastErrorSeq: number;
}

export interface AssistantVoice {
  state: 'off' | 'listening' | 'submitting';
  heard: string;
}

export interface AssistantNote {
  state: 'idle' | 'drafting' | 'ready' | 'saved' | 'failed';
  markdown: string;
  error: string | null;
}

export interface AssistantAskItem {
  id: string;
  question: string;
  // Empty for a For-you item.
  why: string;
}

// A question that left the open list because it was answered. Nothing is
// ever removed from the answered list during a session.
export interface AssistantAnsweredItem {
  // The open item's id when it was on the list, so React keeps its place.
  id: string;
  // The wording Joseph saw on the open list (or asked on the call).
  question: string;
  // The answer, in a sentence or two.
  answer: string;
  // 'call': answered on the call. 'chat': worked out by Joseph and the
  // assistant in Ask. 'you': Joseph typed the answer to a For-you question.
  how: 'call' | 'chat' | 'you';
  // When it was answered, epoch milliseconds.
  at: number;
}

export interface AssistantAsks {
  state: 'idle' | 'thinking' | 'failed' | 'unavailable';
  asks: AssistantAskItem[];
  forYou: AssistantAskItem[];
  // Every question answered this session, newest first.
  answered: AssistantAnsweredItem[];
  updatedAt: number | null;
  brief: string;
  // True while the shown list came from the deep check.
  checked: boolean;
  // True while the shown list is a quick one and the deep check is running.
  checking: boolean;
  // Ids of the items the last deep check added or reworded.
  changed: string[];
}

export interface AssistantSettings {
  enabled: boolean;
  claudePath: string;
  fastModel: string;
  fastEffort: string;
  deepModel: string;
  deepEffort: string;
  triggerMode: 'manual' | 'gated' | 'continuous';
  quietGapSecs: number;
  names: string;
  vaultRoot: string;
  deepReadDirs: string;
  seeZoomWindow: boolean;
}

export interface AssistantClaudeProbe {
  ok: boolean;
  version: string;
  error: string | null;
}
