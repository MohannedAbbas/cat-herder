import { Routes, Route, Navigate } from 'react-router-dom'
import Layout from './components/Layout'
import { AuthProvider, useAuth } from './auth/AuthContext'
import LoginPage from './features/auth/LoginPage'
import SessionListPage from './features/sessions/SessionListPage'
import SessionDetailPage from './features/sessions/SessionDetailPage'
import AgentListPage from './features/agents/AgentListPage'
import AgentDetailPage from './features/agents/AgentDetailPage'
import RunListPage from './features/runs/RunListPage'
import RunDetailPage from './features/runs/RunDetailPage'
import AdminAgentsPage from './features/admin/AdminAgentsPage'
import AdminProvidersPage from './features/admin/AdminProvidersPage'
import AdminModelsPage from './features/admin/AdminModelsPage'
import AdminToolsPage from './features/admin/AdminToolsPage'
import AdminReposPage from './features/admin/AdminReposPage'
import AdminAuditPage from './features/admin/AdminAuditPage'
import AdminWebhooksPage from './features/admin/AdminWebhooksPage'
import AdminWorkflowsPage from './features/admin/AdminWorkflowsPage'

function RequireAdmin({ children }: { children: React.ReactNode }) {
  const { authenticated, loading } = useAuth()
  if (loading) return null
  if (!authenticated) return <Navigate to="/login" replace />
  return <>{children}</>
}

export default function App() {
  return (
    <AuthProvider>
      <Routes>
        <Route element={<Layout />}>
          <Route index element={<Navigate to="/runs" replace />} />
          <Route path="/runs" element={<RunListPage />} />
          <Route path="/runs/:id" element={<RunDetailPage />} />
          <Route
            path="/sessions"
            element={
              <RequireAdmin>
                <SessionListPage />
              </RequireAdmin>
            }
          />
          <Route
            path="/sessions/:id"
            element={
              <RequireAdmin>
                <SessionDetailPage />
              </RequireAdmin>
            }
          />
          <Route
            path="/agents"
            element={
              <RequireAdmin>
                <AgentListPage />
              </RequireAdmin>
            }
          />
          <Route
            path="/agents/:id"
            element={
              <RequireAdmin>
                <AgentDetailPage />
              </RequireAdmin>
            }
          />
          <Route path="/login" element={<LoginPage />} />
          <Route
            path="/admin"
            element={
              <RequireAdmin>
                <Navigate to="/admin/agents" replace />
              </RequireAdmin>
            }
          />
          <Route
            path="/admin/agents"
            element={
              <RequireAdmin>
                <AdminAgentsPage />
              </RequireAdmin>
            }
          />
          <Route
            path="/admin/providers"
            element={
              <RequireAdmin>
                <AdminProvidersPage />
              </RequireAdmin>
            }
          />
          <Route
            path="/admin/models"
            element={
              <RequireAdmin>
                <AdminModelsPage />
              </RequireAdmin>
            }
          />
          <Route
            path="/admin/tools"
            element={
              <RequireAdmin>
                <AdminToolsPage />
              </RequireAdmin>
            }
          />
          <Route
            path="/admin/repos"
            element={
              <RequireAdmin>
                <AdminReposPage />
              </RequireAdmin>
            }
          />
          <Route
            path="/admin/workflows"
            element={
              <RequireAdmin>
                <AdminWorkflowsPage />
              </RequireAdmin>
            }
          />
          <Route
            path="/admin/webhooks"
            element={
              <RequireAdmin>
                <AdminWebhooksPage />
              </RequireAdmin>
            }
          />
          <Route
            path="/admin/audit"
            element={
              <RequireAdmin>
                <AdminAuditPage />
              </RequireAdmin>
            }
          />
        </Route>
      </Routes>
    </AuthProvider>
  )
}
