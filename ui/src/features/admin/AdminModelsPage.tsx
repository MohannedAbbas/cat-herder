import { useState } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import { useModels, useProviders } from '@/hooks/queries'
import PageHeader from '@/components/PageHeader'
import Loading from '@/components/Loading'
import ErrorState from '@/components/ErrorState'
import Card from '@/components/Card'
import type { Model } from '@/types/domain'

const emptyForm = { id: '', providerId: '', displayName: '' }

export default function AdminModelsPage() {
  const { data: models, isLoading, error } = useModels()
  const { data: providers } = useProviders()
  const queryClient = useQueryClient()
  const [editingId, setEditingId] = useState<string | null>(null)
  const [form, setForm] = useState(emptyForm)
  const [message, setMessage] = useState<string | null>(null)

  const save = useMutation({
    mutationFn: (vars: { id: string; model: Model }) => api.saveModel(vars.id, vars.model),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['models'] })
      setEditingId(null)
      setMessage('Saved.')
    },
  })
  const remove = useMutation({
    mutationFn: (id: string) => api.deleteModel(id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['models'] }),
  })

  function startNew() {
    setEditingId('__new__')
    setForm({ ...emptyForm, providerId: providers?.[0]?.id ?? '' })
    setMessage(null)
  }

  function startEdit(m: Model) {
    setEditingId(m.id)
    setForm({ id: m.id, providerId: m.providerId, displayName: m.displayName })
    setMessage(null)
  }

  function submit() {
    if (!editingId || !form.id) return
    const id = form.id
    save.mutate({ id, model: { ...form, id } })
  }

  return (
    <div>
      <PageHeader title="Admin · Models" subtitle="Models within each provider." />
      <div className="mb-4">
        <button onClick={startNew} className="rounded-lg bg-slate-900 px-4 py-2 text-sm font-medium text-white hover:bg-slate-700">
          New model
        </button>
      </div>

      {isLoading && <Loading />}
      {error && <ErrorState message={error.message} />}

      {models && (
        <div className="grid gap-6 lg:grid-cols-3">
          <div className="lg:col-span-2 overflow-hidden rounded-xl border border-slate-200 bg-white shadow-sm">
            <table className="min-w-full divide-y divide-slate-200 text-sm">
              <thead className="bg-slate-50 text-left text-xs font-semibold uppercase tracking-wide text-slate-500">
                <tr>
                  <th className="px-4 py-3">Model</th>
                  <th className="px-4 py-3">Provider</th>
                  <th className="px-4 py-3"></th>
                </tr>
              </thead>
              <tbody className="divide-y divide-slate-100">
                {models.map((m) => (
                  <tr key={m.id} className="hover:bg-slate-50">
                    <td className="px-4 py-3">
                      <div className="font-medium text-slate-900">{m.displayName || m.id}</div>
                      <div className="text-xs text-slate-400">{m.providerId}:{m.id}</div>
                    </td>
                    <td className="px-4 py-3 text-slate-600">{m.providerId}</td>
                    <td className="px-4 py-3 text-right">
                      <button onClick={() => startEdit(m)} className="text-sm text-slate-600 hover:underline">Edit</button>
                      <button onClick={() => remove.mutate(m.id)} className="ml-3 text-sm text-red-600 hover:underline">Delete</button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>

          {editingId && (
            <Card>
              <h2 className="text-sm font-semibold text-slate-900">{editingId === '__new__' ? 'New model' : `Editing ${editingId}`}</h2>
              <div className="mt-4 space-y-3">
                <Field label="ID (e.g. deepseek-chat)">
                  <input value={form.id} disabled={editingId !== '__new__'} onChange={(e) => setForm({ ...form, id: e.target.value })} className={inputCls} />
                </Field>
                <Field label="Provider">
                  <select value={form.providerId} onChange={(e) => setForm({ ...form, providerId: e.target.value })} className={inputCls}>
                    {(providers ?? []).map((p) => (
                      <option key={p.id} value={p.id}>{p.name}</option>
                    ))}
                  </select>
                </Field>
                <Field label="Display name">
                  <input value={form.displayName} onChange={(e) => setForm({ ...form, displayName: e.target.value })} className={inputCls} />
                </Field>
                {message && <p className="text-sm text-emerald-600">{message}</p>}
                <div className="flex gap-2 pt-1">
                  <button onClick={submit} disabled={save.isPending} className="rounded-lg bg-slate-900 px-4 py-2 text-sm font-medium text-white hover:bg-slate-700">Save</button>
                  <button onClick={() => setEditingId(null)} className="rounded-lg px-4 py-2 text-sm text-slate-600 hover:bg-slate-100">Cancel</button>
                </div>
              </div>
            </Card>
          )}
        </div>
      )}
    </div>
  )
}

const inputCls = 'w-full rounded-lg border border-slate-300 px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-slate-400'

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="block">
      <span className="mb-1 block text-sm font-medium text-slate-700">{label}</span>
      {children}
    </label>
  )
}
