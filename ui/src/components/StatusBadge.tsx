import type { RunState } from '@/types/domain'

type Status = RunState | 'idle'

interface Meta {
  label: string
  className: string
  pulse?: boolean
}

const META: Record<Status, Meta> = {
  idle: { label: 'Idle', className: 'bg-slate-100 text-slate-500' },
  queued: { label: 'Queued', className: 'bg-slate-100 text-slate-600' },
  scheduled: { label: 'Scheduled', className: 'bg-slate-100 text-slate-600' },
  pod_pending: { label: 'Pod pending', className: 'bg-amber-50 text-amber-700' },
  running: { label: 'Running', className: 'bg-blue-50 text-blue-700', pulse: true },
  awaiting_input: {
    label: 'Awaiting input',
    className: 'bg-violet-50 text-violet-700',
    pulse: true,
  },
  budget_paused: { label: 'Budget paused', className: 'bg-amber-50 text-amber-700' },
  stuck: { label: 'Stuck', className: 'bg-red-50 text-red-700' },
  awaiting_review: {
    label: 'Awaiting review',
    className: 'bg-cyan-50 text-cyan-700',
    pulse: true,
  },
  reviewed: { label: 'Reviewed', className: 'bg-emerald-50 text-emerald-700' },
  succeeded: { label: 'Succeeded', className: 'bg-emerald-50 text-emerald-700' },
  rejected: { label: 'Rejected', className: 'bg-red-50 text-red-700' },
  failed: { label: 'Failed', className: 'bg-red-50 text-red-700' },
  timed_out: { label: 'Timed out', className: 'bg-orange-50 text-orange-700' },
  cancelled: { label: 'Cancelled', className: 'bg-slate-100 text-slate-500' },
}

export default function StatusBadge({ status }: { status: Status }) {
  const meta = META[status]
  return (
    <span
      className={`inline-flex items-center gap-1.5 whitespace-nowrap rounded-full px-2.5 py-0.5 text-xs font-medium ${meta.className}`}
    >
      {meta.pulse && (
        <span className="relative flex h-1.5 w-1.5">
          <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-current opacity-75" />
          <span className="relative inline-flex h-1.5 w-1.5 rounded-full bg-current" />
        </span>
      )}
      {meta.label}
    </span>
  )
}
