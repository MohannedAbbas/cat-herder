import { Link, useParams } from 'react-router-dom'
import Markdown from 'react-markdown'
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

/** Minimal prose styling for rendered markdown (headings, lists, code, links). */
const markdownClass =
  'text-sm text-slate-700 space-y-3 ' +
  '[&_h1]:text-lg [&_h1]:font-semibold [&_h1]:text-slate-900 ' +
  '[&_h2]:text-base [&_h2]:font-semibold [&_h2]:text-slate-900 ' +
  '[&_h3]:text-sm [&_h3]:font-semibold [&_h3]:text-slate-900 ' +
  '[&_ul]:list-disc [&_ul]:pl-5 [&_ol]:list-decimal [&_ol]:pl-5 [&_li]:my-1 ' +
  '[&_a]:text-blue-600 [&_a]:underline ' +
  '[&_code]:rounded [&_code]:bg-slate-100 [&_code]:px-1 [&_code]:py-0.5 [&_code]:text-xs ' +
  '[&_pre]:overflow-x-auto [&_pre]:rounded-lg [&_pre]:bg-slate-900 [&_pre]:p-3 ' +
  '[&_pre_code]:bg-transparent [&_pre_code]:p-0 [&_pre_code]:text-slate-100 ' +
  '[&_blockquote]:border-l-4 [&_blockquote]:border-slate-200 [&_blockquote]:pl-3 [&_blockquote]:text-slate-500'

function MarkdownBlock({ children }: { children: string }) {
  return (
    <div className={markdownClass}>
      <Markdown>{children}</Markdown>
    </div>
  )
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
            <h2 className="mt-3 text-sm font-semibold text-slate-900">Task spec</h2>
            <div className="mt-1">
              <MarkdownBlock>{run.taskSpec}</MarkdownBlock>
            </div>
          </Card>

          {run.report && run.report.trim().length > 0 && (
            <Card>
              <h2 className="text-sm font-semibold text-slate-900">Report</h2>
              <div className="mt-3">
                <MarkdownBlock>{run.report}</MarkdownBlock>
              </div>
            </Card>
          )}

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
