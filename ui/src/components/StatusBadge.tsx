import type { RunState } from '@/types/domain'

type Status = RunState | 'idle'

interface Meta {
  label: string
  className: string
  pulse?: boolean
}

const META: Record<Status, Meta> = {
  idle: { label: 'Idle', className: 'bg-slate-100 text-slate-500 dark:bg-slate-800 dark:text-slate-400' },
  queued: { label: 'Queued', className: 'bg-slate-100 text-slate-600 dark:bg-slate-800 dark:text-slate-300' },
  scheduled: { label: 'Scheduled', className: 'bg-slate-100 text-slate-600 dark:bg-slate-800 dark:text-slate-300' },
  pod_pending: {
    label: 'Pod pending',
    className: 'bg-amber-50 text-amber-700 dark:bg-amber-950/50 dark:text-amber-300',
  },
  running: {
    label: 'Running',
    className: 'bg-blue-50 text-blue-700 dark:bg-blue-950/50 dark:text-blue-300',
    pulse: true,
  },
  awaiting_input: {
    label: 'Awaiting input',
    className: 'bg-violet-50 text-violet-700 dark:bg-violet-950/50 dark:text-violet-300',
    pulse: true,
  },
  budget_paused: {
    label: 'Budget paused',
    className: 'bg-amber-50 text-amber-700 dark:bg-amber-950/50 dark:text-amber-300',
  },
  stuck: { label: 'Stuck', className: 'bg-red-50 text-red-700 dark:bg-red-950/50 dark:text-red-300' },
  awaiting_review: {
    label: 'Awaiting review',
    className: 'bg-cyan-50 text-cyan-700 dark:bg-cyan-950/50 dark:text-cyan-300',
    pulse: true,
  },
  reviewed: {
    label: 'Reviewed',
    className: 'bg-emerald-50 text-emerald-700 dark:bg-emerald-950/50 dark:text-emerald-300',
  },
  succeeded: {
    label: 'Succeeded',
    className: 'bg-emerald-50 text-emerald-700 dark:bg-emerald-950/50 dark:text-emerald-300',
  },
  rejected: { label: 'Rejected', className: 'bg-red-50 text-red-700 dark:bg-red-950/50 dark:text-red-300' },
  failed: { label: 'Failed', className: 'bg-red-50 text-red-700 dark:bg-red-950/50 dark:text-red-300' },
  timed_out: {
    label: 'Timed out',
    className: 'bg-orange-50 text-orange-700 dark:bg-orange-950/50 dark:text-orange-300',
  },
  cancelled: {
    label: 'Cancelled',
    className: 'bg-slate-100 text-slate-500 dark:bg-slate-800 dark:text-slate-400',
  },
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
