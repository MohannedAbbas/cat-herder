import { useParams } from 'react-router-dom'
import { useAgent } from '@/hooks/queries'
import PageHeader from '@/components/PageHeader'
import Loading from '@/components/Loading'
import ErrorState from '@/components/ErrorState'
import Card from '@/components/Card'

export default function AgentDetailPage() {
  const { id } = useParams<{ id: string }>()
  const { data: agent, isLoading, error } = useAgent(id)

  return (
    <div>
      <PageHeader title={agent?.name ?? 'Agent'} subtitle={agent?.id} />

      {isLoading && <Loading />}
      {error && <ErrorState message={error.message} />}

      {agent && (
        <div className="grid gap-4 lg:grid-cols-2">
          <Card>
            <h2 className="text-sm font-semibold text-slate-900">Configuration</h2>
            <div className="mt-3 space-y-2 text-sm">
              <Row label="Image" value={agent.image} />
              <Row label="Runtime tier" value={agent.runtimeTier} />
              <Row label="Model" value={agent.model} />
              <Row label="Max concurrency" value={String(agent.maxConcurrency)} />
            </div>
          </Card>
          <Card>
            <h2 className="text-sm font-semibold text-slate-900">Tools</h2>
            <div className="mt-3 flex flex-wrap gap-1.5">
              {agent.tools.map((t) => (
                <span
                  key={t}
                  className="rounded bg-slate-100 px-2 py-0.5 text-xs text-slate-600"
                >
                  {t}
                </span>
              ))}
            </div>
          </Card>
        </div>
      )}
    </div>
  )
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex justify-between gap-4">
      <span className="text-slate-500">{label}</span>
      <span className="font-medium text-slate-900">{value}</span>
    </div>
  )
}
