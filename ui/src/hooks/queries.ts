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
