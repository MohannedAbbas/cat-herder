import { Link } from 'react-router-dom'
import { useRuns } from '@/hooks/queries'
import StatusBadge from '@/components/StatusBadge'
import PageHeader from '@/components/PageHeader'
import Loading from '@/components/Loading'
import ErrorState from '@/components/ErrorState'
import { formatDate, formatDuration, formatTokens } from '@/lib/format'
import type { RunRecord } from '@/types/domain'

function triggeredBy(run: RunRecord): string {
  return run.actor ? `${run.trigger} · ${run.actor}` : run.trigger
}

function orDash(value: string | null | undefined): string {
  return value && value.length > 0 ? value : '—'
}

export default function RunListPage() {
  const { data, isLoading, error } = useRuns()

  return (
    <div>
      <PageHeader
        title="Runs"
        subtitle="Every execution — who triggered it, on what model/repo/language, and when it ran."
      />

      {isLoading && <Loading />}
      {error && <ErrorState message={error.message} />}

      {data && (
        <div className="overflow-x-auto rounded-xl border border-slate-200 bg-white shadow-sm dark:border-slate-800 dark:bg-slate-900">
          <table className="min-w-full divide-y divide-slate-200 dark:divide-slate-800 text-sm">
            <thead className="bg-slate-50 text-left text-xs font-semibold uppercase tracking-wide text-slate-500 dark:bg-slate-950/60 dark:text-slate-400">
              <tr>
                <th className="px-4 py-3">ID</th>
                <th className="px-4 py-3">Task</th>
                <th className="px-4 py-3">Status</th>
                <th className="px-4 py-3">Triggered by</th>
                <th className="px-4 py-3">Model</th>
                <th className="px-4 py-3">Repo</th>
                <th className="px-4 py-3">Lang</th>
                <th className="px-4 py-3">Started</th>
                <th className="px-4 py-3">Finished</th>
                <th className="px-4 py-3">Runtime</th>
                <th className="px-4 py-3">Tokens</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-slate-100 dark:divide-slate-800/70">
              {data.map((run) => (
                <tr key={run.id} className="hover:bg-slate-50 dark:hover:bg-slate-800/60">
                  <td className="px-4 py-3 text-slate-400 dark:text-slate-500">#{run.id}</td>
                  <td className="max-w-xs px-4 py-3">
                    <Link
                      to={`/runs/${run.id}`}
                      className="font-medium text-slate-900 dark:text-slate-100 hover:underline"
                      title={run.taskSpec}
                    >
                      <span className="block truncate">{run.taskSpec}</span>
                    </Link>
                    <div className="text-xs uppercase tracking-wide text-slate-400 dark:text-slate-500">
                      {run.kind}
                    </div>
                  </td>
                  <td className="px-4 py-3">
                    <StatusBadge status={run.state} />
                  </td>
                  <td className="px-4 py-3 text-slate-600 dark:text-slate-300">{triggeredBy(run)}</td>
                  <td className="px-4 py-3 text-slate-600 dark:text-slate-300">{orDash(run.model)}</td>
                  <td className="px-4 py-3 text-slate-600 dark:text-slate-300">{orDash(run.repo)}</td>
                  <td className="px-4 py-3 text-slate-600 dark:text-slate-300">{orDash(run.language)}</td>
                  <td className="px-4 py-3 text-slate-500 dark:text-slate-400">{formatDate(run.startedAt)}</td>
                  <td className="px-4 py-3 text-slate-500 dark:text-slate-400">{formatDate(run.finishedAt)}</td>
                  <td className="px-4 py-3 text-slate-500 dark:text-slate-400">
                    {formatDuration(run.startedAt, run.finishedAt)}
                  </td>
                  <td className="px-4 py-3 text-slate-500 dark:text-slate-400">
                    {formatTokens(run.inputTokens + run.outputTokens)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  )
}
