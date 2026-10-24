import { createContext, useContext, useEffect, useState, type ReactNode } from 'react'
import { api } from '@/api/client'
import type { AuthInfo } from '@/api/client'

interface AuthContextValue extends AuthInfo {
  loading: boolean
  login: (username: string, password: string) => Promise<void>
  logout: () => Promise<void>
}

const AuthContext = createContext<AuthContextValue | null>(null)

export function AuthProvider({ children }: { children: ReactNode }) {
  const [info, setInfo] = useState<AuthInfo>({ authenticated: false, username: null })
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    api
      .me()
      .then(setInfo)
      .catch(() => setInfo({ authenticated: false, username: null }))
      .finally(() => setLoading(false))
  }, [])

  const login = async (username: string, password: string) => {
    await api.login(username, password)
    setInfo({ authenticated: true, username })
  }

  const logout = async () => {
    await api.logout()
    setInfo({ authenticated: false, username: null })
  }

  return (
    <AuthContext.Provider value={{ ...info, loading, login, logout }}>
      {children}
    </AuthContext.Provider>
  )
}

export function useAuth(): AuthContextValue {
  const ctx = useContext(AuthContext)
  if (!ctx) throw new Error('useAuth must be used within AuthProvider')
  return ctx
}
