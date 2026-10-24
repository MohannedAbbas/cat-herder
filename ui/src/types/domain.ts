// Domain types mirroring the control-plane data model (PLAN.md §15).

export type RunState =
  | 'queued'
  | 'scheduled'
  | 'pod_pending'
  | 'running'
  | 'awaiting_input'
  | 'budget_paused'
  | 'succeeded'
  | 'awaiting_review'
  | 'reviewed'
  | 'rejected'
  | 'failed'
  | 'timed_out'
  | 'cancelled'
  | 'stuck'

export type RunKind = 'agent' | 'workflow'

export type RuntimeTier = 'default' | 'gvisor' | 'kata'

/** A reusable agent configuration (maps to SandboxTemplate + harness config). */
export interface AgentTemplate {
  id: string
  name: string
  description: string
  image: string
  runtimeTier: RuntimeTier
  model: string
  tools: string[]
  maxConcurrency: number
}

/** A single execution (agent or workflow) belonging to a session. */
export interface Run {
  id: string
  kind: RunKind
  state: RunState
  agentId: string
  agentName: string
  taskSpec: string
  createdAt: string
  startedAt: string | null
  finishedAt: string | null
  costUsd: number | null
}

/** Summary row for a session (list view). */
export interface SessionSummary {
  id: string
  title: string
  createdAt: string
  updatedAt: string
  runCount: number
  /** Aggregate status — the "most active" run state, or 'idle' if no runs. */
  status: RunState | 'idle'
}

/** A session with its full set of runs (detail view). */
export interface Session extends SessionSummary {
  runs: Run[]
}

/**
 * A lifecycle run as returned by `GET /v1/runs` — the richer, run-centric record
 * (trigger context, model/repo/language, and lifecycle timing).
 */
export interface RunRecord {
  id: number
  kind: RunKind
  state: RunState
  trigger: string
  actor: string | null
  model: string | null
  repo: string | null
  language: string | null
  taskSpec: string
  createdAt: string
  startedAt: string | null
  finishedAt: string | null
}
