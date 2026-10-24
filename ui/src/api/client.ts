import type {
  AgentTemplate,
  AuditEntry,
  GitHost,
  GitRepo,
  GitRepoWrite,
  Model,
  Provider,
  RunRecord,
  Session,
  SessionSummary,
  Tool,
} from '@/types/domain'
import { config } from '@/config'
import { mockApi } from './mock'
import { httpApi } from './http'

export interface AuthInfo {
  authenticated: boolean
  username: string | null
}

/** The control-plane API surface consumed by the UI (PLAN.md §14). */
export interface CatHerderApi {
  listSessions(): Promise<SessionSummary[]>
  getSession(id: string): Promise<Session>
  listAgents(): Promise<AgentTemplate[]>
  getAgent(id: string): Promise<AgentTemplate>
  listRuns(): Promise<RunRecord[]>
  getRun(id: number): Promise<RunRecord>

  // auth
  login(username: string, password: string): Promise<void>
  logout(): Promise<void>
  me(): Promise<AuthInfo>

  // admin — agents
  saveAgent(id: string, agent: AgentTemplate): Promise<AgentTemplate>
  deleteAgent(id: string): Promise<void>

  // admin — providers
  listProviders(): Promise<Provider[]>
  saveProvider(id: string, provider: Provider): Promise<Provider>
  deleteProvider(id: string): Promise<void>

  // admin — models
  listModels(): Promise<Model[]>
  saveModel(id: string, model: Model): Promise<Model>
  deleteModel(id: string): Promise<void>

  // admin — tools
  listTools(): Promise<Tool[]>
  saveTool(id: string, tool: Tool): Promise<Tool>
  deleteTool(id: string): Promise<void>

  // admin — repos
  listRepos(): Promise<GitRepo[]>
  listGitHosts(): Promise<GitHost[]>
  saveRepo(repo: GitRepoWrite): Promise<GitRepo>
  deleteRepo(id: string): Promise<void>

  // admin — webhook secret
  getWebhookSecret(): Promise<{ secret: string | null }>
  regenerateWebhookSecret(): Promise<{ secret: string }>

  // admin — encryption key
  rotateEncryptionKey(): Promise<void>

  // admin — audit
  listAudit(): Promise<AuditEntry[]>
}

/**
 * The active API implementation. Mock is the default until the control-plane
 * REST endpoints are implemented; set `VITE_USE_MOCK=false` to hit the real API.
 */
export const api: CatHerderApi = config.useMock ? mockApi : httpApi
