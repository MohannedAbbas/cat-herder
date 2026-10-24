import { useAudit } from '@/hooks/queries'
import PageHeader from '@/components/PageHeader'
import Loading from '@/components/Loading'
import ErrorState from '@/components/ErrorState'
import { formatDate } from '@/lib/format'

export default function AdminAuditPage() {
  const { data, isLoading, error } = useAudit()

  return (
    <div>
      <PageHeader title="Admin · Audit" subtitle="Recent admin actions." />

      {isLoading && <Loading />}
      {error && <ErrorState message={error.message} />}

      {data && (
        <div className="overflow-x-auto rounded-xl border border-slate-200 bg-white shadow-sm">
          <table className="min-w-full divide-y divide-slate-200 text-sm">
            <thead className="bg-slate-50 text-left text-xs font-semibold uppercase tracking-wide text-slate-500">
              <tr>
                <th className="px-4 py-3">When</th>
                <th className="px-4 py-3">Actor</th>
                <th className="px-4 py-3">Action</th>
                <th className="px-4 py-3">Resource</th>
                <th className="px-4 py-3">ID</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-slate-100">
              {data.length === 0 && (
                <tr>
                  <td colSpan={5} className="px-4 py-8 text-center text-slate-400">
                    No admin actions yet.
                  </td>
                </tr>
              )}
              {data.map((e) => (
                <tr key={e.id} className="hover:bg-slate-50">
                  <td className="whitespace-nowrap px-4 py-3 text-slate-500">{formatDate(e.timestamp)}</td>
                  <td className="px-4 py-3 text-slate-600">{e.actor}</td>
                  <td className="px-4 py-3">
                    <span className="rounded bg-slate-100 px-2 py-0.5 font-mono text-xs text-slate-700">
                      {e.action}
                    </span>
                  </td>
                  <td className="px-4 py-3 text-slate-600">{e.resource}</td>
                  <td className="px-4 py-3 font-mono text-xs text-slate-500">{e.resourceId}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  )
}
