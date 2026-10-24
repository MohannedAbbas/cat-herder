import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import PageHeader from '@/components/PageHeader'
import Loading from '@/components/Loading'
import ErrorState from '@/components/ErrorState'
import Card from '@/components/Card'

export default function AdminWebhooksPage() {
  const queryClient = useQueryClient()
  const { data, isLoading, error } = useQuery({
    queryKey: ['webhook-secret'],
    queryFn: () => api.getWebhookSecret(),
  })
  const regenerate = useMutation({
    mutationFn: () => api.regenerateWebhookSecret(),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['webhook-secret'] }),
  })

  const secret = data?.secret ?? null

  function copy() {
    if (secret) navigator.clipboard.writeText(secret)
  }

  return (
    <div>
      <PageHeader
        title="Admin · Webhooks"
        subtitle="Shared secret required to trigger runs via the webhook endpoint."
      />

      {isLoading && <Loading />}
      {error && <ErrorState message={error.message} />}

      {data && (
        <div className="grid gap-4 lg:grid-cols-2">
          <Card>
            <h2 className="text-sm font-semibold text-slate-900">Webhook secret</h2>
            <p className="mt-1 text-sm text-slate-500">
              Send this in the <code className="font-mono text-slate-700">X-Webhook-Secret</code>{' '}
              header on every webhook call. Requests without a matching secret are rejected.
            </p>

            <div className="mt-3 flex items-center gap-2">
              <code className="flex-1 break-all rounded bg-slate-100 px-3 py-2 font-mono text-sm text-slate-800">
                {secret ?? 'Not configured'}
              </code>
              {secret && (
                <button
                  onClick={copy}
                  className="rounded-lg border border-slate-300 px-3 py-2 text-sm text-slate-600 hover:bg-slate-100"
                >
                  Copy
                </button>
              )}
            </div>

            <button
              onClick={() => regenerate.mutate()}
              disabled={regenerate.isPending}
              className="mt-3 rounded-lg bg-slate-900 px-4 py-2 text-sm font-medium text-white hover:bg-slate-700 disabled:opacity-50"
            >
              {regenerate.isPending ? 'Regenerating…' : secret ? 'Regenerate' : 'Generate'}
            </button>
          </Card>

          <Card>
            <h2 className="text-sm font-semibold text-slate-900">Example call</h2>
            <pre className="mt-3 overflow-x-auto rounded bg-slate-900 p-3 text-xs leading-relaxed text-slate-100">
{`curl -X POST http://localhost:8080/v1/webhooks/generic \\
  -H 'Content-Type: application/json' \\
  -H 'X-Webhook-Secret: ${secret ?? '<secret>'} ' \\
  -d '{
    "task_spec": "Fix the login bug",
    "repo": "https://github.com/org/repo.git",
    "dedup_key": "delivery-42"
  }'`}
            </pre>
          </Card>
        </div>
      )}
    </div>
  )
}
