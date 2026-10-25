import type {
  AgentTemplate,
  AuditEntry,
  GitHost,
  GitRepo,
  Model,
  Provider,
  RunRecord,
  Session,
  SessionSummary,
  Tool,
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
