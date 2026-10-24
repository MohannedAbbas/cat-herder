import type { AgentTemplate, RunRecord, Session, SessionSummary } from '@/types/domain'
import { config } from '@/config'
import { mockApi } from './mock'
import { httpApi } from './http'

/** The control-plane API surface consumed by the UI (PLAN.md §14). */
export interface CatHerderApi {
  listSessions(): Promise<SessionSummary[]>
  getSession(id: string): Promise<Session>
  listAgents(): Promise<AgentTemplate[]>
  getAgent(id: string): Promise<AgentTemplate>
  listRuns(): Promise<RunRecord[]>
  getRun(id: number): Promise<RunRecord>
}

/**
 * The active API implementation. Mock is the default until the control-plane
 * REST endpoints are implemented; set `VITE_USE_MOCK=false` to hit the real API.
 */
export const api: CatHerderApi = config.useMock ? mockApi : httpApi
