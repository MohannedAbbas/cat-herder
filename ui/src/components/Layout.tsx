import { Link, NavLink, Outlet, useNavigate } from 'react-router-dom'
import { useAuth } from '@/auth/AuthContext'

const navLinkClass = ({ isActive }: { isActive: boolean }) =>
  `block rounded-md px-3 py-2 text-sm font-medium transition ${
    isActive
      ? 'bg-slate-900 text-white'
      : 'text-slate-600 hover:bg-slate-100 hover:text-slate-900'
  }`

export default function Layout() {
  const { authenticated, username, logout } = useAuth()
  const navigate = useNavigate()

  async function onLogout() {
    await logout()
    navigate('/runs')
  }

  return (
    <div className="min-h-screen bg-slate-50">
      <div className="flex">
        <aside className="sticky top-0 flex h-screen w-60 shrink-0 flex-col border-r border-slate-200 bg-white p-4">
          <Link to="/" className="mb-6 flex items-center gap-2 px-2">
            <span className="flex h-8 w-8 items-center justify-center rounded-lg bg-slate-900 text-sm font-bold text-white">
              CH
            </span>
            <span className="text-lg font-semibold text-slate-900">Cat Herder</span>
          </Link>
          <nav className="flex flex-col gap-1">
            <NavLink to="/runs" className={navLinkClass}>
              Runs
            </NavLink>
            <NavLink to="/sessions" className={navLinkClass}>
              Sessions
            </NavLink>
            <NavLink to="/agents" className={navLinkClass}>
              Agents
            </NavLink>

            {authenticated && (
              <>
                <div className="mt-4 mb-1 px-2 text-xs font-semibold uppercase tracking-wide text-slate-400">
                  Admin
                </div>
                <NavLink to="/admin/agents" className={navLinkClass}>
                  Agents
                </NavLink>
                <NavLink to="/admin/providers" className={navLinkClass}>
                  Providers
                </NavLink>
                <NavLink to="/admin/models" className={navLinkClass}>
                  Models
                </NavLink>
              </>
            )}
          </nav>

          <div className="mt-auto border-t border-slate-200 pt-3">
            {authenticated ? (
              <div className="flex items-center justify-between px-2">
                <span className="text-sm text-slate-600">{username}</span>
                <button onClick={onLogout} className="text-sm text-slate-500 hover:underline">
                  Sign out
                </button>
              </div>
            ) : (
              <NavLink to="/login" className={navLinkClass}>
                Admin sign in
              </NavLink>
            )}
          </div>
        </aside>
        <main className="min-w-0 flex-1 p-8">
          <Outlet />
        </main>
      </div>
    </div>
  )
}
