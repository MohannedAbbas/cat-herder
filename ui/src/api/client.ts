import type {
  AgentTemplate,
  Approval,
  AuditEntry,
  GitHost,
  GitRepo,
  GitRepoWrite,
  Model,
  Provider,
  Review,
  ReviewVerdict,
  RunDiff,
  RunLogLine,
  RunRecord,
  Session,
  SessionSummary,
  Tool,
  WorkflowDefinition,
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

  // workflows (§7)
  listWorkflows(): Promise<WorkflowDefinition[]>
  saveWorkflow(id: string, workflow: WorkflowDefinition): Promise<WorkflowDefinition>
  deleteWorkflow(id: string): Promise<void>

  // run deep-link views (§10, §20) — de-blind approvals and reviews
  getRunDiff(id: number): Promise<RunDiff>
  /** The pending approval for a run, or `null` when there is none. */
  getRunApproval(id: number): Promise<Approval | null>
  /** The pending review for a run, or `null` when there is none. */
  getRunReview(id: number): Promise<Review | null>
  respondApproval(id: string, approve: boolean, response?: string): Promise<void>
  respondReview(id: string, verdict: ReviewVerdict, feedback?: string): Promise<void>

  // live run logs (§16, NEXT_STEPS #18)
  /** The buffered run logs as a JSON array (no live tail). */
  getRunLogs(id: number): Promise<RunLogLine[]>
  /**
   * Tail a run's logs. Calls `onLine` for each buffered line then each live line, and
   * returns a function that stops following. Best-effort: mock mode yields a few
   * sample lines.
   */
  followRunLogs(id: number, onLine: (line: RunLogLine) => void): () => void

  // auth
  login(username: string, password: string): Promise<void>
  logout(): Promise<void>
  me(): Promise<AuthInfo>
  /** SSO providers enabled on the control plane, e.g. `['github', 'gitlab']`. */
  ssoProviders(): Promise<string[]>

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
