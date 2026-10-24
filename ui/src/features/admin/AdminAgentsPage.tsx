import { useState } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import { useAgents, useModels, useProviders, useTools } from '@/hooks/queries'
import PageHeader from '@/components/PageHeader'
import Loading from '@/components/Loading'
import ErrorState from '@/components/ErrorState'
import Card from '@/components/Card'
import type { AgentTemplate, Model, RuntimeTier } from '@/types/domain'

const emptyForm = {
  name: '',
  description: '',
  model: '',
  runtimeTier: 'default' as RuntimeTier,
  tools: [] as string[],
  maxConcurrency: 8,
}

function groupModelsByProvider(models: Model[]): [string, Model[]][] {
  const map = new Map<string, Model[]>()
  for (const m of models) {
    const list = map.get(m.providerId) ?? []
    list.push(m)
    map.set(m.providerId, list)
  }
  return Array.from(map.entries())
}

export default function AdminAgentsPage() {
  const { data: agents, isLoading, error } = useAgents()
  const { data: models } = useModels()
  const { data: providers } = useProviders()
  const { data: tools } = useTools()
  const queryClient = useQueryClient()
  const [editingId, setEditingId] = useState<string | null>(null)
  const [form, setForm] = useState(emptyForm)
  const [message, setMessage] = useState<string | null>(null)

  const save = useMutation({
    mutationFn: (vars: { id: string; agent: AgentTemplate }) => api.saveAgent(vars.id, vars.agent),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['agents'] })
      setEditingId(null)
      setMessage('Saved.')
    },
  })
  const remove = useMutation({
    mutationFn: (id: string) => api.deleteAgent(id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['agents'] }),
  })

  const providerName = (id: string) => providers?.find((p) => p.id === id)?.name ?? id

  function startEdit(agent: AgentTemplate) {
    setEditingId(agent.id)
    setForm({
      name: agent.name,
      description: agent.description,
      model: agent.model,
      runtimeTier: agent.runtimeTier,
      tools: agent.tools,
      maxConcurrency: agent.maxConcurrency,
    })
    setMessage(null)
  }

  function submit() {
    if (!editingId) return
    const id = editingId
    save.mutate({
      id,
      agent: {
        id,
        name: form.name,
        description: form.description,
        model: form.model,
        image: 'cat-herder-agent-harness:0.1.0',
        runtimeTier: form.runtimeTier,
        tools: form.tools,
        maxConcurrency: form.maxConcurrency,
      },
    })
  }

  return (
    <div>
      <PageHeader title="Admin · Agents" subtitle="Edit agent templates and their models." />
      {isLoading && <Loading />}
      {error && <ErrorState message={error.message} />}

      {agents && (
        <div className="grid gap-6 lg:grid-cols-3">
          <div className="lg:col-span-2 overflow-hidden rounded-xl border border-slate-200 bg-white shadow-sm">
            <table className="min-w-full divide-y divide-slate-200 text-sm">
              <thead className="bg-slate-50 text-left text-xs font-semibold uppercase tracking-wide text-slate-500">
                <tr>
                  <th className="px-4 py-3">Agent</th>
                  <th className="px-4 py-3">Model</th>
                  <th className="px-4 py-3"></th>
                </tr>
              </thead>
              <tbody className="divide-y divide-slate-100">
                {agents.map((a) => (
                  <tr key={a.id} className="hover:bg-slate-50">
                    <td className="px-4 py-3">
                      <div className="font-medium text-slate-900">{a.name}</div>
                      <div className="text-xs text-slate-400">{a.id}</div>
                    </td>
                    <td className="px-4 py-3 text-slate-600">{a.model}</td>
                    <td className="px-4 py-3 text-right">
                      <button onClick={() => startEdit(a)} className="text-sm text-slate-600 hover:underline">
                        Edit
                      </button>
                      <button
                        onClick={() => remove.mutate(a.id)}
                        className="ml-3 text-sm text-red-600 hover:underline"
                      >
                        Delete
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>

          {editingId && (
            <Card>
              <h2 className="text-sm font-semibold text-slate-900">Editing {editingId}</h2>
              <div className="mt-4 space-y-3">
                <Field label="Name">
                  <input value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} className={inputCls} />
                </Field>
                <Field label="Description">
                  <textarea value={form.description} onChange={(e) => setForm({ ...form, description: e.target.value })} className={inputCls} rows={2} />
                </Field>

                <Field label="Model">
                  <select value={form.model} onChange={(e) => setForm({ ...form, model: e.target.value })} className={inputCls}>
                    <option value="">— none —</option>
                    {groupModelsByProvider(models ?? []).map(([providerId, ms]) => (
                      <optgroup key={providerId} label={providerName(providerId)}>
                        {ms.map((m) => (
                          <option key={`${m.providerId}:${m.id}`} value={`${m.providerId}:${m.id}`}>
                            {m.displayName || m.id}
                          </option>
                        ))}
                      </optgroup>
                    ))}
                  </select>
                </Field>

                <Field label="Tools">
                  <div className="max-h-52 overflow-y-auto rounded-lg border border-slate-300 p-2">
                    {(tools ?? []).map((t) => (
                      <label key={t.id} className="flex cursor-pointer items-center gap-2 py-1 text-sm">
                        <input
                          type="checkbox"
                          checked={form.tools.includes(t.id)}
                          onChange={(e) => {
                            const selected = e.target.checked
                              ? [...form.tools, t.id]
                              : form.tools.filter((x) => x !== t.id)
                            setForm({ ...form, tools: selected })
                          }}
                        />
                        <span className="font-mono text-slate-700">{t.id}</span>
                        {t.description && <span className="truncate text-xs text-slate-400">{t.description}</span>}
                      </label>
                    ))}
                    {(tools ?? []).length === 0 && (
                      <p className="py-1 text-sm text-slate-400">No tools defined — add some in the Tools tab.</p>
                    )}
                  </div>
                </Field>

                <Field label="Max concurrency">
                  <input
                    type="number"
                    value={form.maxConcurrency}
                    onChange={(e) => setForm({ ...form, maxConcurrency: Number(e.target.value) })}
                    className={inputCls}
                  />
                </Field>
                {message && <p className="text-sm text-emerald-600">{message}</p>}
                <div className="flex gap-2 pt-1">
                  <button onClick={submit} disabled={save.isPending} className="rounded-lg bg-slate-900 px-4 py-2 text-sm font-medium text-white hover:bg-slate-700">
                    Save
                  </button>
                  <button onClick={() => setEditingId(null)} className="rounded-lg px-4 py-2 text-sm text-slate-600 hover:bg-slate-100">
                    Cancel
                  </button>
                </div>
              </div>
            </Card>
          )}
        </div>
      )}
    </div>
  )
}

const inputCls =
  'w-full rounded-lg border border-slate-300 px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-slate-400'

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="block">
      <span className="mb-1 block text-sm font-medium text-slate-700">{label}</span>
      {children}
    </label>
  )
}
