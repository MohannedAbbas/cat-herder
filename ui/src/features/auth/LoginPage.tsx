import { useEffect, useState } from 'react'
import { Navigate, useNavigate } from 'react-router-dom'
import { api } from '@/api/client'
import { useAuth } from '@/auth/AuthContext'
import Card from '@/components/Card'

/** Display labels for the SSO providers the control plane may report. */
const PROVIDER_LABELS: Record<string, string> = {
  github: 'GitHub',
  gitlab: 'GitLab',
}

export default function LoginPage() {
  const { authenticated, login } = useAuth()
  const navigate = useNavigate()
  const [username, setUsername] = useState('admin')
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [submitting, setSubmitting] = useState(false)
  const [providers, setProviders] = useState<string[]>([])

  // Ask the control plane which SSO providers are configured so we render the
  // matching buttons. Failures are non-fatal: password login still works.
  useEffect(() => {
    let active = true
    api
      .ssoProviders()
      .then((list) => {
        if (active) setProviders(list)
      })
      .catch(() => {
        if (active) setProviders([])
      })
    return () => {
      active = false
    }
  }, [])

  if (authenticated) return <Navigate to="/admin" replace />

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault()
    setSubmitting(true)
    setError(null)
    try {
      await login(username, password)
      navigate('/admin')
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Login failed')
    } finally {
      setSubmitting(false)
    }
  }

  // Full-page redirect into the control plane's OAuth flow; the callback sets the
  // session cookie and bounces the browser back to "/".
  function signInWith(provider: string) {
    window.location.href = `/v1/auth/${provider}/login`
  }

  return (
    <div className="flex min-h-[70vh] items-center justify-center">
      <Card className="w-full max-w-sm">
        <h1 className="text-lg font-semibold text-slate-900">Admin sign in</h1>
        <p className="mt-1 text-sm text-slate-500">Manage agents, models, and providers.</p>

        <form onSubmit={onSubmit} className="mt-6 space-y-4">
          <div>
            <label className="mb-1 block text-sm font-medium text-slate-700">Username</label>
            <input
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              className="w-full rounded-lg border border-slate-300 px-3 py-2 text-sm"
              autoComplete="username"
            />
          </div>
          <div>
            <label className="mb-1 block text-sm font-medium text-slate-700">Password</label>
            <input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              className="w-full rounded-lg border border-slate-300 px-3 py-2 text-sm"
              autoComplete="current-password"
            />
          </div>
          {error && <p className="text-sm text-red-600">{error}</p>}
          <button
            type="submit"
            disabled={submitting}
            className="w-full rounded-lg bg-slate-900 px-4 py-2 text-sm font-medium text-white hover:bg-slate-700 disabled:opacity-50"
          >
            {submitting ? 'Signing in…' : 'Sign in'}
          </button>
        </form>

        {providers.length > 0 && (
          <div className="mt-6">
            <div className="relative">
              <div className="absolute inset-0 flex items-center">
                <div className="w-full border-t border-slate-200" />
              </div>
              <div className="relative flex justify-center text-xs uppercase">
                <span className="bg-white px-2 text-slate-400">or sign in with</span>
              </div>
            </div>
            <div className="mt-4 space-y-2">
              {providers.map((provider) => {
                const label = PROVIDER_LABELS[provider] ?? provider
                return (
                  <button
                    key={provider}
                    type="button"
                    onClick={() => signInWith(provider)}
                    className="w-full rounded-lg border border-slate-300 px-4 py-2 text-sm font-medium text-slate-800 hover:bg-slate-50"
                  >
                    Sign in with {label}
                  </button>
                )
              })}
            </div>
          </div>
        )}
      </Card>
    </div>
  )
}
