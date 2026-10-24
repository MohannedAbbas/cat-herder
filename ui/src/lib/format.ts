const dateFormatter = new Intl.DateTimeFormat(undefined, {
  dateStyle: 'medium',
  timeStyle: 'short',
})

export function formatDate(iso: string | null | undefined): string {
  if (!iso) return '—'
  return dateFormatter.format(new Date(iso))
}

export function formatCost(usd: number | null | undefined): string {
  if (usd == null) return '—'
  return usd.toLocaleString(undefined, {
    style: 'currency',
    currency: 'USD',
    maximumFractionDigits: 4,
  })
}

/**
 * Human-readable duration between two ISO timestamps. If `endIso` is null, the
 * duration is measured up to now (for in-flight runs).
 */
export function formatDuration(
  startIso: string | null | undefined,
  endIso: string | null | undefined,
): string {
  if (!startIso) return '—'
  const start = new Date(startIso).getTime()
  const end = endIso ? new Date(endIso).getTime() : Date.now()
  const seconds = Math.max(0, Math.floor((end - start) / 1000))
  if (seconds < 60) return `${seconds}s`
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`
  const hours = Math.floor(minutes / 60)
  return `${hours}h ${minutes % 60}m`
}
