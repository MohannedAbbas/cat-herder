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
