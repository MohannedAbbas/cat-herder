import type { AgentTemplate, RunRecord, Session, SessionSummary } from '@/types/domain'
import type { CatHerderApi } from './client'
import { config } from '@/config'

async function get<T>(path: string): Promise<T> {
  const res = await fetch(`${config.apiBaseUrl}${path}`)
  if (!res.ok) {
    throw new Error(`GET ${path} failed: ${res.status} ${res.statusText}`)
  }
  return (await res.json()) as T
}

/** Real HTTP client against the control-plane REST API (used when mock is off). */
export const httpApi: CatHerderApi = {
  listSessions: () => get<SessionSummary[]>('/v1/sessions'),
  getSession: (id) => get<Session>(`/v1/sessions/${id}`),
  listAgents: () => get<AgentTemplate[]>('/v1/agents'),
  getAgent: (id) => get<AgentTemplate>(`/v1/agents/${id}`),
  listRuns: () => get<RunRecord[]>('/v1/runs'),
  getRun: (id) => get<RunRecord>(`/v1/runs/${id}`),
}
