import { Link, useParams } from 'react-router-dom'
import { useRun } from '@/hooks/queries'
import StatusBadge from '@/components/StatusBadge'
import PageHeader from '@/components/PageHeader'
import Loading from '@/components/Loading'
import ErrorState from '@/components/ErrorState'
import Card from '@/components/Card'
import { formatDate, formatDuration } from '@/lib/format'

function orDash(value: string | null | undefined): string {
  return value && value.length > 0 ? value : '—'
}

export default function RunDetailPage() {
  const { id } = useParams<{ id: string }>()
  const { data: run, isLoading, error } = useRun(id)

  return (
    <div>
      <PageHeader
        title={run ? `Run #${run.id}` : 'Run'}
        subtitle={run ? run.taskSpec : undefined}
      />

      {isLoading && <Loading />}
      {error && <ErrorState message={error.message} />}

      {run && (
        <div className="space-y-4">
          <Card>
            <div className="flex items-center gap-3">
              <StatusBadge status={run.state} />
              <span className="text-xs uppercase tracking-wide text-slate-400">{run.kind}</span>
              <span className="text-xs text-slate-400">Runtime {formatDuration(run.startedAt, run.finishedAt)}</span>
            </div>
            <p className="mt-3 text-sm font-medium text-slate-900">{run.taskSpec}</p>
          </Card>

          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
            <Card>
              <h2 className="text-sm font-semibold text-slate-900">Trigger</h2>
              <div className="mt-3 space-y-2 text-sm">
                <Row label="Channel" value={run.trigger} />
                <Row label="Actor" value={orDash(run.actor)} />
                <Row
                  label="Session"
                  value={
                    run.sessionId ? (
                      <Link
                        to={`/sessions/${run.sessionId}`}
                        className="font-medium text-slate-900 hover:underline"
                      >
                        {run.sessionId}
                      </Link>
                    ) : (
                      '—'
                    )
                  }
                />
              </div>
            </Card>

            <Card>
              <h2 className="text-sm font-semibold text-slate-900">Context</h2>
              <div className="mt-3 space-y-2 text-sm">
                <Row label="Model" value={orDash(run.model)} />
                <Row label="Repository" value={orDash(run.repo)} />
                <Row
                  label="Pull request"
                  value={
                    run.prUrl ? (
                      <a
                        href={run.prUrl}
                        target="_blank"
                        rel="noreferrer"
                        className="font-medium text-blue-600 hover:underline"
                      >
                        {run.prUrl}
                      </a>
                    ) : (
                      '—'
                    )
                  }
                />
                <Row label="Language" value={orDash(run.language)} />
              </div>
            </Card>

            <Card>
              <h2 className="text-sm font-semibold text-slate-900">Timing</h2>
              <div className="mt-3 space-y-2 text-sm">
                <Row label="Triggered" value={formatDate(run.createdAt)} />
                <Row label="Started" value={formatDate(run.startedAt)} />
                <Row label="Finished" value={formatDate(run.finishedAt)} />
              </div>
            </Card>
          </div>
        </div>
      )}
    </div>
  )
}

function Row({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="flex justify-between gap-4">
      <span className="shrink-0 text-slate-500">{label}</span>
      <span className="min-w-0 truncate text-right font-medium text-slate-900">{value}</span>
    </div>
  )
}
