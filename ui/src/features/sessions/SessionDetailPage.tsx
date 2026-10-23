import { Link, useParams } from 'react-router-dom'
import { useSession } from '@/hooks/queries'
import StatusBadge from '@/components/StatusBadge'
import PageHeader from '@/components/PageHeader'
import Loading from '@/components/Loading'
import ErrorState from '@/components/ErrorState'
import Card from '@/components/Card'
import { formatCost, formatDate } from '@/lib/format'
import type { Run } from '@/types/domain'

export default function SessionDetailPage() {
  const { id } = useParams<{ id: string }>()
  const { data: session, isLoading, error } = useSession(id)

  return (
    <div>
      <PageHeader
        title={session?.title ?? 'Session'}
        subtitle={session ? `Session ${session.id} · ${session.runCount} runs` : undefined}
      />

      {isLoading && <Loading />}
      {error && <ErrorState message={error.message} />}

      {session && (
        <div className="space-y-3">
          {session.runs.map((run) => (
            <RunRow key={run.id} run={run} />
          ))}
        </div>
      )}
    </div>
  )
}

function RunRow({ run }: { run: Run }) {
  return (
    <Card>
      <div className="flex items-center gap-2">
        <StatusBadge status={run.state} />
        <span className="text-xs uppercase tracking-wide text-slate-400">{run.kind}</span>
      </div>
      <p className="mt-2 text-sm font-medium text-slate-900">{run.taskSpec}</p>
      <div className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-slate-500">
        <Link to={`/agents/${run.agentId}`} className="text-slate-600 hover:underline">
          {run.agentName}
        </Link>
        <span>Started {formatDate(run.startedAt)}</span>
        <span>Finished {formatDate(run.finishedAt)}</span>
        <span>Cost {formatCost(run.costUsd)}</span>
      </div>
    </Card>
  )
}
