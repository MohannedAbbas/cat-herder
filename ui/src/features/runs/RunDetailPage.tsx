import { useState } from 'react'
import { Link, useParams } from 'react-router-dom'
import Markdown from 'react-markdown'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import { useRun, useRunApproval, useRunDiff, useRunReview } from '@/hooks/queries'
import StatusBadge from '@/components/StatusBadge'
import PageHeader from '@/components/PageHeader'
import Loading from '@/components/Loading'
import ErrorState from '@/components/ErrorState'
import Card from '@/components/Card'
import { formatDate, formatDuration, formatTokens } from '@/lib/format'
import type { Approval, Review, ReviewVerdict, RunDiff } from '@/types/domain'

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

function fileStatusClass(status: string): string {
  if (status === 'added') return 'rounded-full bg-emerald-50 px-2 py-0.5 text-xs text-emerald-700'
  if (status === 'deleted') return 'rounded-full bg-red-50 px-2 py-0.5 text-xs text-red-700'
  return 'rounded-full bg-amber-50 px-2 py-0.5 text-xs text-amber-700'
}

/** The workspace diff — the changed files a reviewer must see before responding. */
function DiffPanel({ diff }: { diff: RunDiff }) {
  if (diff.changes.length === 0) return null
  return (
    <Card>
      <div className="flex items-baseline justify-between">
        <h2 className="text-sm font-semibold text-slate-900">Workspace diff</h2>
        <span className="text-xs text-slate-400">
          {diff.changes.length} file{diff.changes.length === 1 ? '' : 's'} changed
        </span>
      </div>
      <div className="mt-3 space-y-3">
        {diff.changes.map((fc) => (
          <div key={fc.path} className="overflow-hidden rounded-lg border border-slate-200">
            <div className="flex items-center justify-between gap-3 border-b border-slate-100 bg-slate-50 px-3 py-2">
              <code className="truncate font-mono text-xs text-slate-700">{fc.path}</code>
              <span className={`shrink-0 ${fileStatusClass(fc.status)}`}>{fc.status}</span>
            </div>
            {fc.content != null ? (
              <pre className="max-h-80 overflow-auto p-3 font-mono text-xs leading-relaxed text-slate-700">
                {fc.content}
              </pre>
            ) : (
              <div className="px-3 py-2 text-xs italic text-slate-400">No content (deleted file)</div>
            )}
          </div>
        ))}
      </div>
    </Card>
  )
}

/** The pending approval — the interrupt checkpoint and pending tool call the approver
 * must see before approving or rejecting. */
function ApprovalPanel({ approval, runId }: { approval: Approval; runId: number }) {
  const queryClient = useQueryClient()
  const [response, setResponse] = useState('')
  const [error, setError] = useState<string | null>(null)

  const decide = useMutation({
    mutationFn: (approve: boolean) => api.respondApproval(approval.id, approve, response || undefined),
    onSuccess: () => {
      setError(null)
      queryClient.invalidateQueries({ queryKey: ['runs', runId] })
      queryClient.invalidateQueries({ queryKey: ['runs', runId, 'approval'] })
    },
    onError: (err: Error) => setError(err.message),
  })

  return (
    <Card className="border-violet-200">
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold text-slate-900">Awaiting your approval</h2>
        <span className="font-mono text-xs text-slate-400">checkpoint {approval.resume_token.slice(0, 8)}…</span>
      </div>

      <div className="mt-3">
        <div className="text-xs font-medium uppercase tracking-wide text-slate-400">Pending action</div>
        <pre className="mt-1 overflow-x-auto rounded-lg bg-slate-900 p-3 text-xs text-slate-100">
          {JSON.stringify(approval.payload, null, 2)}
        </pre>
      </div>

      <div className="mt-3">
        <label className="block">
          <span className="mb-1 block text-xs font-medium uppercase tracking-wide text-slate-400">
            Response (optional)
          </span>
          <textarea
            value={response}
            onChange={(e) => setResponse(e.target.value)}
            rows={2}
            placeholder="A note to send back to the agent (e.g. 'go ahead')"
            className="w-full rounded-lg border border-slate-300 px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-violet-400"
          />
        </label>
      </div>

      {error && <p className="mt-2 text-sm text-red-600">{error}</p>}

      <div className="mt-3 flex gap-2">
        <button
          onClick={() => decide.mutate(true)}
          disabled={decide.isPending}
          className="rounded-lg bg-slate-900 px-4 py-2 text-sm font-medium text-white hover:bg-slate-700 disabled:opacity-50"
        >
          {decide.isPending ? 'Saving…' : 'Approve'}
        </button>
        <button
          onClick={() => decide.mutate(false)}
          disabled={decide.isPending}
          className="rounded-lg border border-red-300 px-4 py-2 text-sm font-medium text-red-700 hover:bg-red-50 disabled:opacity-50"
        >
          Reject
        </button>
      </div>
    </Card>
  )
}

/** The pending review — the reviewer's verdict options and feedback box. */
function ReviewPanel({ review, runId }: { review: Review; runId: number }) {
  const queryClient = useQueryClient()
  const [feedback, setFeedback] = useState('')
  const [error, setError] = useState<string | null>(null)

  const decide = useMutation({
    mutationFn: (verdict: ReviewVerdict) => api.respondReview(review.id, verdict, feedback || undefined),
    onSuccess: () => {
      setError(null)
      queryClient.invalidateQueries({ queryKey: ['runs', runId] })
      queryClient.invalidateQueries({ queryKey: ['runs', runId, 'review'] })
    },
    onError: (err: Error) => setError(err.message),
  })

  return (
    <Card className="border-cyan-200">
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold text-slate-900">Awaiting your review</h2>
        <span className="text-xs text-slate-400">due {formatDate(new Date(review.deadline_at).toISOString())}</span>
      </div>

      <div className="mt-3">
        <label className="block">
          <span className="mb-1 block text-xs font-medium uppercase tracking-wide text-slate-400">
            Feedback (required for change requests)
          </span>
          <textarea
            value={feedback}
            onChange={(e) => setFeedback(e.target.value)}
            rows={3}
            placeholder="What should change before this is accepted?"
            className="w-full rounded-lg border border-slate-300 px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-cyan-400"
          />
        </label>
      </div>

      {error && <p className="mt-2 text-sm text-red-600">{error}</p>}

      <div className="mt-3 flex flex-wrap gap-2">
        <button
          onClick={() => decide.mutate('approved')}
          disabled={decide.isPending}
          className="rounded-lg bg-emerald-600 px-4 py-2 text-sm font-medium text-white hover:bg-emerald-700 disabled:opacity-50"
        >
          Approve
        </button>
        <button
          onClick={() => decide.mutate('changes_requested')}
          disabled={decide.isPending}
          className="rounded-lg bg-amber-500 px-4 py-2 text-sm font-medium text-white hover:bg-amber-600 disabled:opacity-50"
        >
          Request changes
        </button>
        <button
          onClick={() => decide.mutate('rejected')}
          disabled={decide.isPending}
          className="rounded-lg border border-red-300 px-4 py-2 text-sm font-medium text-red-700 hover:bg-red-50 disabled:opacity-50"
        >
          Reject
        </button>
      </div>
    </Card>
  )
}

export default function RunDetailPage() {
  const { id } = useParams<{ id: string }>()
  const { data: run, isLoading, error } = useRun(id)
  const { data: diff } = useRunDiff(id)
  const { data: approval } = useRunApproval(id)
  const { data: review } = useRunReview(id)

  return (
    <div>
      <PageHeader title={run ? `Run #${run.id}` : 'Run'} />

      {isLoading && <Loading />}
      {error && <ErrorState message={error.message} />}

      {run && (
        <div className="space-y-4">
          {run.state === 'awaiting_input' && approval && (
            <ApprovalPanel approval={approval} runId={run.id} />
          )}
          {run.state === 'awaiting_review' && review && (
            <ReviewPanel review={review} runId={run.id} />
          )}

          {diff && diff.changes.length > 0 && <DiffPanel diff={diff} />}

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

            <Card>
              <h2 className="text-sm font-semibold text-slate-900">Usage</h2>
              <div className="mt-3 space-y-2 text-sm">
                <Row label="Input tokens" value={formatTokens(run.inputTokens)} />
                <Row label="Output tokens" value={formatTokens(run.outputTokens)} />
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
