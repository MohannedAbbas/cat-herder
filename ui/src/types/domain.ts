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
  inputTokens: number
  outputTokens: number
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
  sessionId: string | null
  taskSpec: string
  createdAt: string
  startedAt: string | null
  finishedAt: string | null
  prUrl: string | null
  /** The agent's final markdown report (from the harness `finished` event). */
  report: string | null
  /** LLM token usage (from the harness `finished` event). */
  inputTokens: number
  outputTokens: number
}

/** An LLM provider. The API key is referenced by Secret name, never returned. */
export interface Provider {
  id: string
  name: string
  baseUrl: string
  apiKeySecret: string
  apiKeySecretKey: string
  enabled: boolean
}

/** A model within a provider. Full model string is `providerId:id`. */
export interface Model {
  id: string
  providerId: string
  displayName: string
}

/** An admin action recorded in the audit log. */
export interface AuditEntry {
  id: number
  timestamp: string
  actor: string
  action: string
  resource: string
  resourceId: string
}

/** An available agent tool (referenced by id in agent templates). */
export interface Tool {
  id: string
  description: string
}

/** A git host option (GitHub only for now), driven by the settings store. */
export interface GitHost {
  host: string
  label: string
}

/** A git repository an agent can clone. The tokens are never returned; `hasToken` /
 * `hasWriteToken` report whether each is stored (encrypted) server-side. */
export interface GitRepo {
  id: string
  name: string
  host: string
  url: string
  enabled: boolean
  hasToken: boolean
  hasWriteToken: boolean
}

/** Write model for a git repo. `host` + `repo` (path) are required; the canonical
 * id/name/url are derived server-side. Both tokens are write-only. */
export interface GitRepoWrite {
  host: string
  repo: string
  enabled: boolean
  token?: string
  writeToken?: string
}

/** A changed file in a run's workspace diff (`GET /v1/runs/{id}/diff`). */
export interface FileChange {
  path: string
  /** `added` | `modified` | `deleted`. */
  status: string
  /** Decoded file content (text), or `null` for deleted / binary files. */
  content: string | null
}

/** The workspace diff for a run. */
export interface RunDiff {
  changes: FileChange[]
}

export type ReviewVerdict = 'approved' | 'rejected' | 'changes_requested'

/** A pending result review (`GET /v1/runs/{id}/review`). */
export interface Review {
  id: string
  run_id: number
  status: string
  verdict: ReviewVerdict | null
  reviewer: string | null
  feedback: string | null
  created_at: number
  deadline_at: number
}

/** A pending human-in-the-loop approval (`GET /v1/runs/{id}/approval`). */
export interface Approval {
  id: string
  run_id: number
  status: string
  /** The opaque LangGraph checkpoint id of the paused state. */
  resume_token: string
  /** The pending tool call / question shown to the approver. */
  payload: unknown
  created_at: number
}
