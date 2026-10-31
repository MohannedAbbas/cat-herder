import { useQuery } from '@tanstack/react-query'
import { api } from '@/api/client'

export function useSessions() {
  return useQuery({ queryKey: ['sessions'], queryFn: () => api.listSessions() })
}

export function useSession(id: string | undefined) {
  return useQuery({
    queryKey: ['sessions', id],
    queryFn: () => api.getSession(id!),
    enabled: id != null,
  })
}

export function useAgents() {
  return useQuery({ queryKey: ['agents'], queryFn: () => api.listAgents() })
}

export function useAgent(id: string | undefined) {
  return useQuery({
    queryKey: ['agents', id],
    queryFn: () => api.getAgent(id!),
    enabled: id != null,
  })
}

export function useRuns() {
  return useQuery({ queryKey: ['runs'], queryFn: () => api.listRuns() })
}

export function useRun(id: string | undefined) {
  return useQuery({
    queryKey: ['runs', id],
    queryFn: () => api.getRun(Number(id)),
    enabled: id != null,
  })
}

export function useRunDiff(id: string | undefined) {
  return useQuery({
    queryKey: ['runs', id, 'diff'],
    queryFn: () => api.getRunDiff(Number(id)),
    enabled: id != null,
  })
}

export function useRunApproval(id: string | undefined) {
  return useQuery({
    queryKey: ['runs', id, 'approval'],
    queryFn: () => api.getRunApproval(Number(id)),
    enabled: id != null,
  })
}

export function useRunReview(id: string | undefined) {
  return useQuery({
    queryKey: ['runs', id, 'review'],
    queryFn: () => api.getRunReview(Number(id)),
    enabled: id != null,
  })
}

export function useProviders() {
  return useQuery({ queryKey: ['providers'], queryFn: () => api.listProviders() })
}

export function useModels() {
  return useQuery({ queryKey: ['models'], queryFn: () => api.listModels() })
}

export function useAudit() {
  return useQuery({ queryKey: ['audit'], queryFn: () => api.listAudit() })
}

export function useTools() {
  return useQuery({ queryKey: ['tools'], queryFn: () => api.listTools() })
}

export function useRepos() {
  return useQuery({ queryKey: ['repos'], queryFn: () => api.listRepos() })
}

export function useGitHosts() {
  return useQuery({ queryKey: ['gitHosts'], queryFn: () => api.listGitHosts() })
}
