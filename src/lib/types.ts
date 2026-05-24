export type Status = 'running' | 'finished' | 'stopped';
export type DisplayKind = 'current' | 'recent' | 'longest';
export type Source = 'codex' | 'claude';

export interface Segment {
  start: string;
  end: string;
  duration_secs: number;
  mode: string | null;
  turn_count: number;
}

export interface Goal {
  objective: string;
  status: string | null;
  time_used_seconds: number | null;
}

export interface Session {
  source: Source;
  id: string;
  cwd: string | null;
  git_branch: string | null;
  cli_version: string | null;
  model: string | null;
  reasoning_effort: string | null;
  current_mode: string | null;
  has_plan_mode: boolean;
  goal: Goal | null;
  started_at: string;
  last_event_at: string;
  total_turns: number;
  total_duration_secs: number;
  segment_count: number;
  segments: Segment[];
  status: Status;
  display_segment: Segment;
  display_segment_kind: DisplayKind;
  file_path: string;
  file_size_bytes: number;
}
