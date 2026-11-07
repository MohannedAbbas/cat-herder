import { Link } from 'react-router-dom'
import { useAgents } from '@/hooks/queries'
import PageHeader from '@/components/PageHeader'
import Loading from '@/components/Loading'
import ErrorState from '@/components/ErrorState'
import Card from '@/components/Card'

export default function AgentListPage() {
  const { data, isLoading, error } = useAgents()

  return (
    <div>
      <PageHeader
        title="Agents"
        subtitle="Reusable agent templates (SandboxTemplate + harness config)."
      />

      {isLoading && <Loading />}
      {error && <ErrorState message={error.message} />}

      {data && (
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
          {data.map((a) => (
            <Link key={a.id} to={`/agents/${a.id}`} className="group">
              <Card className="h-full transition group-hover:border-slate-300 dark:border-slate-800 dark:group-hover:border-slate-600">
                <div className="text-sm font-semibold text-slate-900 dark:text-slate-100">{a.name}</div>
                <div className="mt-1 text-xs text-slate-400 dark:text-slate-500">{a.id}</div>
                <p className="mt-2 text-sm text-slate-600 dark:text-slate-300">{a.description}</p>
                <div className="mt-3 flex flex-wrap gap-1.5">
                  <span className="rounded bg-slate-100 px-2 py-0.5 text-xs text-slate-600 dark:bg-slate-800 dark:text-slate-300">
                    {a.model}
                  </span>
                  <span className="rounded bg-slate-100 px-2 py-0.5 text-xs text-slate-600 dark:bg-slate-800 dark:text-slate-300">
                    {a.runtimeTier}
                  </span>
                </div>
              </Card>
            </Link>
          ))}
        </div>
      )}
    </div>
  )
}
