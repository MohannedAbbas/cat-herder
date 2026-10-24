import { Routes, Route, Navigate } from 'react-router-dom'
import Layout from './components/Layout'
import SessionListPage from './features/sessions/SessionListPage'
import SessionDetailPage from './features/sessions/SessionDetailPage'
import AgentListPage from './features/agents/AgentListPage'
import AgentDetailPage from './features/agents/AgentDetailPage'
import RunListPage from './features/runs/RunListPage'

export default function App() {
  return (
    <Routes>
      <Route element={<Layout />}>
        <Route index element={<Navigate to="/runs" replace />} />
        <Route path="/runs" element={<RunListPage />} />
        <Route path="/sessions" element={<SessionListPage />} />
        <Route path="/sessions/:id" element={<SessionDetailPage />} />
        <Route path="/agents" element={<AgentListPage />} />
        <Route path="/agents/:id" element={<AgentDetailPage />} />
      </Route>
    </Routes>
  )
}
