import type {
  AgentTemplate,
  AuditEntry,
  Model,
  Provider,
  Run,
  RunRecord,
  RunState,
  Session,
  SessionSummary,
  Tool,
} from '@/types/domain'
import type { CatHerderApi, AuthInfo } from './client'

let authenticated = false

let auditLog: AuditEntry[] = []

let auditSeq = 0
function recordAudit(action: string, resource: string, resourceId: string) {
  auditLog = [
    {
      id: ++auditSeq,
      timestamp: new Date().toISOString(),
      actor: 'admin',
      action,
      resource,
      resourceId,
    },
    ...auditLog,
  ]
}

let agents: AgentTemplate[] = [
  {
    id: 'agent-codeact',
    name: 'CodeAct coding agent',
    description: 'General-purpose coding agent: plans, edits, tests, and opens PRs.',
    image: 'cat-herder-agent-harness:0.1.0',
    runtimeTier: 'default',
    model: 'anthropic:claude-sonnet-4-6',
    tools: ['filesystem', 'execute', 'git', 'github', 'credential'],
    maxConcurrency: 8,
  },
  {
    id: 'agent-reviewer',
    name: 'Code review agent',
    description: 'Reviews diffs for correctness, security, and style.',
    image: 'cat-herder-agent-harness:0.1.0',
    runtimeTier: 'gvisor',
    model: 'openai:gpt-5.5',
    tools: ['filesystem', 'git', 'github'],
    maxConcurrency: 4,
  },
  {
    id: 'agent-docs',
    name: 'Documentation agent',
    description: 'Generates changelogs and API docs from the codebase.',
    image: 'cat-herder-agent-harness:0.1.0',
    runtimeTier: 'default',
    model: 'openai:gpt-5.5',
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
    sessionId: 'sess-01',
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
    sessionId: 'sess-02',
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
    sessionId: 'sess-03',
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
    sessionId: 'sess-04',
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

let providers: Provider[] = [
  { id: 'deepseek', name: 'DeepSeek', baseUrl: 'https://api.deepseek.com', apiKeySecret: 'cat-herder-deepseek', apiKeySecretKey: 'api-key', enabled: true },
  { id: 'openai', name: 'OpenAI', baseUrl: 'https://api.openai.com/v1', apiKeySecret: 'cat-herder-openai', apiKeySecretKey: 'api-key', enabled: true },
  { id: 'anthropic', name: 'Anthropic', baseUrl: 'https://api.anthropic.com', apiKeySecret: 'cat-herder-anthropic', apiKeySecretKey: 'api-key', enabled: true },
]

let models: Model[] = [
  { id: 'deepseek-chat', providerId: 'deepseek', displayName: 'DeepSeek Chat (V3)' },
  { id: 'deepseek-reasoner', providerId: 'deepseek', displayName: 'DeepSeek Reasoner (R1)' },
  { id: 'gpt-5.5', providerId: 'openai', displayName: 'GPT-5.5' },
  { id: 'gpt-4o', providerId: 'openai', displayName: 'GPT-4o' },
  { id: 'claude-sonnet-4-6', providerId: 'anthropic', displayName: 'Claude Sonnet 4.6' },
]

let tools: Tool[] = [
  { id: 'filesystem', description: 'Read/write files in the workspace' },
  { id: 'execute', description: 'Run shell commands' },
  { id: 'git', description: 'Git operations' },
  { id: 'github', description: 'GitHub issues and PRs' },
  { id: 'credential', description: 'Scoped credential brokering' },
  { id: 'web_search', description: 'Web search' },
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
  getRun: (id) => {
    const r = runs.find((x) => x.id === id)
    return r ? delay(r) : Promise.reject(new Error(`run ${id} not found`))
  },

  login: (_u, _p) => {
    authenticated = true
    return delay(undefined)
  },
  logout: () => {
    authenticated = false
    return delay(undefined)
  },
  me: () => delay<AuthInfo>({ authenticated, username: authenticated ? 'admin' : null }),

  saveAgent: (id, agent) => {
    const i = agents.findIndex((a) => a.id === id)
    const saved = { ...agent, id }
    if (i >= 0) agents[i] = saved
    else agents.push(saved)
    recordAudit('agent.upsert', 'agent', id)
    return delay(saved)
  },
  deleteAgent: (id) => {
    agents = agents.filter((a) => a.id !== id)
    recordAudit('agent.delete', 'agent', id)
    return delay(undefined)
  },

  listProviders: () => delay(providers),
  saveProvider: (id, provider) => {
    const i = providers.findIndex((p) => p.id === id)
    const saved = { ...provider, id }
    if (i >= 0) providers[i] = saved
    else providers.push(saved)
    recordAudit('provider.upsert', 'provider', id)
    return delay(saved)
  },
  deleteProvider: (id) => {
    providers = providers.filter((p) => p.id !== id)
    recordAudit('provider.delete', 'provider', id)
    return delay(undefined)
  },

  listModels: () => delay(models),
  saveModel: (id, model) => {
    const i = models.findIndex((m) => m.id === id)
    const saved = { ...model, id }
    if (i >= 0) models[i] = saved
    else models.push(saved)
    recordAudit('model.upsert', 'model', id)
    return delay(saved)
  },
  deleteModel: (id) => {
    models = models.filter((m) => m.id !== id)
    recordAudit('model.delete', 'model', id)
    return delay(undefined)
  },

  listTools: () => delay(tools),
  saveTool: (id, tool) => {
    const i = tools.findIndex((t) => t.id === id)
    const saved = { ...tool, id }
    if (i >= 0) tools[i] = saved
    else tools.push(saved)
    recordAudit('tool.upsert', 'tool', id)
    return delay(saved)
  },
  deleteTool: (id) => {
    tools = tools.filter((t) => t.id !== id)
    recordAudit('tool.delete', 'tool', id)
    return delay(undefined)
  },

  listAudit: () => delay(auditLog),
}
