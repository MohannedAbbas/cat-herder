import type {
  AgentTemplate,
  Approval,
  AuditEntry,
  GitHost,
  GitRepo,
  Model,
  Provider,
  Review,
  RunDiff,
  RunRecord,
  Session,
  SessionSummary,
  Tool,
  WorkflowDefinition,
} from '@/types/domain'
import type { CatHerderApi, AuthInfo } from './client'
import { config } from '@/config'

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${config.apiBaseUrl}${path}`, {
    credentials: 'include',
    ...init,
  })
  if (!res.ok) {
    const body = await res.text().catch(() => '')
    const detail = body ? `: ${body}` : ''
    throw new Error(`${init?.method ?? 'GET'} ${path} failed: ${res.status} ${res.statusText}${detail}`)
  }
  if (res.status === 204) return undefined as T
  return (await res.json()) as T
}

/** Like `request`, but returns `null` for a 404 (used for optional per-run views). */
async function requestOptional<T>(path: string): Promise<T | null> {
  const res = await fetch(`${config.apiBaseUrl}${path}`, { credentials: 'include' })
  if (res.status === 404) return null
  if (!res.ok) {
    const body = await res.text().catch(() => '')
    throw new Error(`GET ${path} failed: ${res.status} ${res.statusText}${body ? `: ${body}` : ''}`)
  }
  return (await res.json()) as T
}

function json(method: string, body?: unknown): RequestInit {
  return {
    method,
    headers: { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  }
}

/** Real HTTP client against the control-plane REST API (used when mock is off). */
export const httpApi: CatHerderApi = {
  listSessions: () => request<SessionSummary[]>('/v1/sessions'),
  getSession: (id) => request<Session>(`/v1/sessions/${id}`),
  listAgents: () => request<AgentTemplate[]>('/v1/agents'),
  getAgent: (id) => request<AgentTemplate>(`/v1/agents/${id}`),
  listRuns: () => request<RunRecord[]>('/v1/runs'),
  getRun: (id) => request<RunRecord>(`/v1/runs/${id}`),

  listWorkflows: () => request<WorkflowDefinition[]>('/v1/workflows'),
  saveWorkflow: (id, workflow) =>
    request<WorkflowDefinition>(`/v1/workflows/${id}`, json('PUT', workflow)),
  deleteWorkflow: (id) =>
    request<void>(`/v1/workflows/${id}`, { method: 'DELETE' }),

  getRunDiff: (id) => request<RunDiff>(`/v1/runs/${id}/diff`),
  getRunApproval: (id) => requestOptional<Approval>(`/v1/runs/${id}/approval`),
  getRunReview: (id) => requestOptional<Review>(`/v1/runs/${id}/review`),
  respondApproval: (id, approve, response) =>
    request(`/v1/approvals/${id}/respond`, json('POST', { approve, response })).then(
      () => undefined,
    ),
  respondReview: (id, verdict, feedback) =>
    request(`/v1/reviews/${id}/respond`, json('POST', { verdict, feedback })).then(
      () => undefined,
    ),

  login: (username, password) =>
    request('/v1/auth/login', json('POST', { username, password })).then(() => undefined),
  logout: () => request('/v1/auth/logout', json('POST')).then(() => undefined),
  me: () => request<AuthInfo>('/v1/auth/me'),
  ssoProviders: () =>
    request<{ providers: string[] }>('/v1/auth/sso-providers').then((r) => r.providers ?? []),

  saveAgent: (id, agent) => request<AgentTemplate>(`/v1/admin/agents/${id}`, json('PUT', agent)),
  deleteAgent: (id) => request<void>(`/v1/admin/agents/${id}`, { method: 'DELETE' }),

  listProviders: () => request<Provider[]>('/v1/admin/providers'),
  saveProvider: (id, provider) =>
    request<Provider>(`/v1/admin/providers/${id}`, json('PUT', provider)),
  deleteProvider: (id) => request<void>(`/v1/admin/providers/${id}`, { method: 'DELETE' }),

  listModels: () => request<Model[]>('/v1/admin/models'),
  saveModel: (id, model) => request<Model>(`/v1/admin/models/${id}`, json('PUT', model)),
  deleteModel: (id) => request<void>(`/v1/admin/models/${id}`, { method: 'DELETE' }),

  listTools: () => request<Tool[]>('/v1/admin/tools'),
  saveTool: (id, tool) => request<Tool>(`/v1/admin/tools/${id}`, json('PUT', tool)),
  deleteTool: (id) => request<void>(`/v1/admin/tools/${id}`, { method: 'DELETE' }),

  listRepos: () => request<GitRepo[]>('/v1/admin/repos'),
  listGitHosts: () => request<GitHost[]>('/v1/admin/git-hosts'),
  saveRepo: (repo) => request<GitRepo>('/v1/admin/repos', json('PUT', repo)),
  deleteRepo: (id) => request<void>(`/v1/admin/repos?id=${encodeURIComponent(id)}`, { method: 'DELETE' }),

  getWebhookSecret: () =>
    request<{ secret: string | null }>('/v1/admin/settings/webhook-secret'),
  regenerateWebhookSecret: () =>
    request<{ secret: string }>('/v1/admin/settings/webhook-secret', { method: 'POST' }),

  rotateEncryptionKey: () =>
    request('/v1/admin/settings/encryption-key/rotate', { method: 'POST' }).then(() => undefined),

  listAudit: () => request<AuditEntry[]>('/v1/admin/audit'),
}
