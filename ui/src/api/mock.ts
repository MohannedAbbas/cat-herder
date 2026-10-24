import type { AgentTemplate, Run, RunRecord, RunState, Session, SessionSummary } from '@/types/domain'
import type { CatHerderApi } from './client'

const agents: AgentTemplate[] = [
  {
    id: 'agent-codeact',
    name: 'CodeAct coding agent',
    description: 'General-purpose coding agent: plans, edits, tests, and opens PRs.',
    image: 'cat-herder-agent-harness:0.1.0',
    runtimeTier: 'default',
    model: 'anthropic/claude-sonnet-4-6',
    tools: ['filesystem', 'execute', 'git', 'github', 'credential'],
    maxConcurrency: 8,
  },
  {
    id: 'agent-reviewer',
    name: 'Code review agent',
    description: 'Reviews diffs for correctness, security, and style.',
    image: 'cat-herder-agent-harness:0.1.0',
    runtimeTier: 'gvisor',
    model: 'openai/gpt-5.5',
    tools: ['filesystem', 'git', 'github'],
    maxConcurrency: 4,
  },
  {
    id: 'agent-docs',
    name: 'Documentation agent',
    description: 'Generates changelogs and API docs from the codebase.',
    image: 'cat-herder-agent-harness:0.1.0',
    runtimeTier: 'default',
    model: 'openai/gpt-5.5',
    tools: ['filesystem', 'web_search'],
    maxConcurrency: 4,
  },
]

const sessions: Session[] = [
  {
    id: 'sess-01',
    title: 'Fix auth token refresh bug',
    createdAt: '2026-10-23T09:14:00Z',
    updatedAt: '2026-10-23T10:02:00Z',
    runCount: 3,
    status: 'running',
    runs: [
      {
        id: 'run-101',
        kind: 'agent',
        state: 'running',
        agentId: 'agent-codeact',
        agentName: 'CodeAct coding agent',
        taskSpec: 'Investigate and fix the token refresh race in auth middleware.',
        createdAt: '2026-10-23T09:15:00Z',
        startedAt: '2026-10-23T09:16:00Z',
        finishedAt: null,
        costUsd: 1.23,
      },
      {
        id: 'run-102',
        kind: 'agent',
        state: 'awaiting_input',
        agentId: 'agent-codeact',
        agentName: 'CodeAct coding agent',
        taskSpec: 'Push the fix to main and open a PR.',
        createdAt: '2026-10-23T09:58:00Z',
        startedAt: '2026-10-23T09:59:00Z',
        finishedAt: null,
        costUsd: 0.41,
      },
      {
        id: 'run-103',
        kind: 'agent',
        state: 'succeeded',
        agentId: 'agent-reviewer',
        agentName: 'Code review agent',
        taskSpec: 'Review the auth fix diff for security issues.',
        createdAt: '2026-10-23T09:40:00Z',
        startedAt: '2026-10-23T09:41:00Z',
        finishedAt: '2026-10-23T09:52:00Z',
        costUsd: 0.67,
      },
    ],
  },
  {
    id: 'sess-02',
    title: 'Migrate to axum 0.8',
    createdAt: '2026-10-22T15:30:00Z',
    updatedAt: '2026-10-22T16:45:00Z',
    runCount: 2,
    status: 'failed',
    runs: [
      {
        id: 'run-201',
        kind: 'agent',
        state: 'failed',
        agentId: 'agent-codeact',
        agentName: 'CodeAct coding agent',
        taskSpec: 'Migrate the HTTP adapter from axum 0.7 to 0.8.',
        createdAt: '2026-10-22T15:31:00Z',
        startedAt: '2026-10-22T15:32:00Z',
        finishedAt: '2026-10-22T16:12:00Z',
        costUsd: 3.88,
      },
      {
        id: 'run-202',
        kind: 'workflow',
        state: 'cancelled',
        agentId: 'agent-reviewer',
        agentName: 'Code review agent',
        taskSpec: 'Verify the axum migration against the canary suite.',
        createdAt: '2026-10-22T16:20:00Z',
        startedAt: null,
        finishedAt: '2026-10-22T16:45:00Z',
        costUsd: null,
      },
    ],
  },
  {
    id: 'sess-03',
    title: 'Generate v0.1.0 release notes',
    createdAt: '2026-10-21T11:00:00Z',
    updatedAt: '2026-10-21T11:38:00Z',
    runCount: 2,
    status: 'awaiting_review',
    runs: [
      {
        id: 'run-301',
        kind: 'agent',
        state: 'awaiting_review',
        agentId: 'agent-docs',
        agentName: 'Documentation agent',
        taskSpec: 'Generate release notes from merged PRs since v0.0.9.',
        createdAt: '2026-10-21T11:01:00Z',
        startedAt: '2026-10-21T11:02:00Z',
        finishedAt: '2026-10-21T11:35:00Z',
        costUsd: 0.92,
      },
      {
        id: 'run-302',
        kind: 'agent',
        state: 'succeeded',
        agentId: 'agent-docs',
        agentName: 'Documentation agent',
        taskSpec: 'Update the README quickstart to match v0.1.0.',
        createdAt: '2026-10-21T11:20:00Z',
        startedAt: '2026-10-21T11:21:00Z',
        finishedAt: '2026-10-21T11:38:00Z',
        costUsd: 0.35,
      },
    ],
  },
  {
    id: 'sess-04',
    title: 'Investigate flaky e2e test',
    createdAt: '2026-10-20T08:05:00Z',
    updatedAt: '2026-10-20T08:05:00Z',
    runCount: 1,
    status: 'stuck',
    runs: [
      {
        id: 'run-401',
        kind: 'agent',
        state: 'stuck',
        agentId: 'agent-codeact',
        agentName: 'CodeAct coding agent',
        taskSpec: 'Reproduce and diagnose the flaky workspace snapshot test.',
        createdAt: '2026-10-20T08:05:00Z',
        startedAt: '2026-10-20T08:06:00Z',
        finishedAt: null,
        costUsd: 2.11,
      },
    ],
  },
]

const runs: RunRecord[] = [
  {
    id: 1,
    kind: 'agent',
    state: 'running',
    trigger: 'webhook',
    actor: 'octocat',
    model: 'anthropic/claude-sonnet-4-6',
    repo: 'org/auth-service',
    language: 'rust',
    taskSpec: 'Investigate and fix the token refresh race in auth middleware.',
    createdAt: '2026-10-23T09:15:00Z',
    startedAt: '2026-10-23T09:16:00Z',
    finishedAt: null,
  },
  {
    id: 2,
    kind: 'agent',
    state: 'succeeded',
    trigger: 'api',
    actor: null,
    model: 'openai/gpt-5.5',
    repo: 'org/web-app',
    language: 'typescript',
    taskSpec: 'Migrate the HTTP adapter from axum 0.7 to 0.8.',
    createdAt: '2026-10-22T15:31:00Z',
    startedAt: '2026-10-22T15:32:00Z',
    finishedAt: '2026-10-22T16:12:00Z',
  },
  {
    id: 3,
    kind: 'workflow',
    state: 'failed',
    trigger: 'mcp',
    actor: 'mohanned',
    model: 'openai/gpt-5.5',
    repo: 'org/release-tooling',
    language: 'python',
    taskSpec: 'Generate release notes from merged PRs since v0.0.9.',
    createdAt: '2026-10-21T11:01:00Z',
    startedAt: '2026-10-21T11:02:00Z',
    finishedAt: '2026-10-21T11:35:00Z',
  },
  {
    id: 4,
    kind: 'agent',
    state: 'queued',
    trigger: 'webhook',
    actor: null,
    model: null,
    repo: 'org/infra',
    language: 'go',
    taskSpec: 'Reproduce and diagnose the flaky workspace snapshot test.',
    createdAt: '2026-10-20T08:05:00Z',
    startedAt: null,
    finishedAt: null,
  },
]

// Active → terminal priority for deriving a session's aggregate status.
const STATUS_PRIORITY: RunState[] = [
  'running',
  'awaiting_input',
  'pod_pending',
  'scheduled',
  'queued',
  'budget_paused',
  'stuck',
  'awaiting_review',
  'timed_out',
  'failed',
  'rejected',
  'cancelled',
  'reviewed',
  'succeeded',
]

function aggregateStatus(runs: Run[]): RunState | 'idle' {
  if (runs.length === 0) return 'idle'
  const present = new Set(runs.map((r) => r.state))
  return STATUS_PRIORITY.find((s) => present.has(s)) ?? 'succeeded'
}

function toSummary(s: Session): SessionSummary {
  return {
    id: s.id,
    title: s.title,
    createdAt: s.createdAt,
    updatedAt: s.updatedAt,
    runCount: s.runs.length,
    status: aggregateStatus(s.runs),
  }
}

function delay<T>(value: T, ms = 250): Promise<T> {
  return new Promise((resolve) => setTimeout(() => resolve(value), ms))
}

export const mockApi: CatHerderApi = {
  listSessions: () => delay(sessions.map(toSummary)),
  getSession: (id) => {
    const s = sessions.find((x) => x.id === id)
    return s ? delay(s) : Promise.reject(new Error(`session ${id} not found`))
  },
  listAgents: () => delay(agents),
  getAgent: (id) => {
    const a = agents.find((x) => x.id === id)
    return a ? delay(a) : Promise.reject(new Error(`agent ${id} not found`))
  },
  listRuns: () => delay(runs),
}
