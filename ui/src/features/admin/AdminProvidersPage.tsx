import { useState } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import { useProviders } from '@/hooks/queries'
import PageHeader from '@/components/PageHeader'
import Loading from '@/components/Loading'
import ErrorState from '@/components/ErrorState'
import Card from '@/components/Card'
import type { Provider } from '@/types/domain'

const emptyForm = {
  id: '',
  name: '',
  baseUrl: '',
  apiKeySecret: '',
  apiKeySecretKey: 'api-key',
  enabled: true,
}

export default function AdminProvidersPage() {
  const { data: providers, isLoading, error } = useProviders()
  const queryClient = useQueryClient()
  const [editingId, setEditingId] = useState<string | null>(null)
  const [form, setForm] = useState(emptyForm)
  const [message, setMessage] = useState<string | null>(null)

  const save = useMutation({
    mutationFn: (vars: { id: string; provider: Provider }) => api.saveProvider(vars.id, vars.provider),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['providers'] })
      setEditingId(null)
      setMessage('Saved.')
    },
  })
  const remove = useMutation({
    mutationFn: (id: string) => api.deleteProvider(id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['providers'] }),
  })

  function startNew() {
    setEditingId('__new__')
    setForm(emptyForm)
    setMessage(null)
  }

  function startEdit(p: Provider) {
    setEditingId(p.id)
    setForm({ id: p.id, name: p.name, baseUrl: p.baseUrl, apiKeySecret: p.apiKeySecret, apiKeySecretKey: p.apiKeySecretKey, enabled: p.enabled })
    setMessage(null)
  }

  function submit() {
    if (!editingId || !form.id) return
    const id = form.id
    save.mutate({ id, provider: { ...form, id, enabled: form.enabled } })
  }

  return (
    <div>
      <PageHeader title="Admin · Providers" subtitle="LLM providers and where their API keys live." />
      <div className="mb-4">
        <button onClick={startNew} className="rounded-lg bg-slate-900 px-4 py-2 text-sm font-medium text-white hover:bg-slate-700">
          New provider
        </button>
      </div>

      {isLoading && <Loading />}
      {error && <ErrorState message={error.message} />}

      {providers && (
        <div className="grid gap-6 lg:grid-cols-3">
          <div className="lg:col-span-2 overflow-hidden rounded-xl border border-slate-200 bg-white shadow-sm">
            <table className="min-w-full divide-y divide-slate-200 text-sm">
              <thead className="bg-slate-50 text-left text-xs font-semibold uppercase tracking-wide text-slate-500">
                <tr>
                  <th className="px-4 py-3">Provider</th>
                  <th className="px-4 py-3">Base URL</th>
                  <th className="px-4 py-3">Key Secret</th>
                  <th className="px-4 py-3"></th>
                </tr>
              </thead>
              <tbody className="divide-y divide-slate-100">
                {providers.map((p) => (
                  <tr key={p.id} className="hover:bg-slate-50">
                    <td className="px-4 py-3">
                      <div className="font-medium text-slate-900">{p.name}</div>
                      <div className="text-xs text-slate-400">{p.id}</div>
                    </td>
                    <td className="px-4 py-3 text-slate-600">{p.baseUrl}</td>
                    <td className="px-4 py-3 text-slate-600">
                      {p.apiKeySecret}/{p.apiKeySecretKey}
                    </td>
                    <td className="px-4 py-3 text-right">
                      <button onClick={() => startEdit(p)} className="text-sm text-slate-600 hover:underline">Edit</button>
                      <button onClick={() => remove.mutate(p.id)} className="ml-3 text-sm text-red-600 hover:underline">Delete</button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>

          {editingId && (
            <Card>
              <h2 className="text-sm font-semibold text-slate-900">{editingId === '__new__' ? 'New provider' : `Editing ${editingId}`}</h2>
              <div className="mt-4 space-y-3">
                <Field label="ID (e.g. deepseek)">
                  <input value={form.id} disabled={editingId !== '__new__'} onChange={(e) => setForm({ ...form, id: e.target.value })} className={inputCls} />
                </Field>
                <Field label="Name">
                  <input value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} className={inputCls} />
                </Field>
                <Field label="Base URL">
                  <input value={form.baseUrl} onChange={(e) => setForm({ ...form, baseUrl: e.target.value })} className={inputCls} />
                </Field>
                <Field label="API key Secret name">
                  <input value={form.apiKeySecret} onChange={(e) => setForm({ ...form, apiKeySecret: e.target.value })} className={inputCls} />
                </Field>
                <Field label="Secret key">
                  <input value={form.apiKeySecretKey} onChange={(e) => setForm({ ...form, apiKeySecretKey: e.target.value })} className={inputCls} />
                </Field>
                <label className="flex items-center gap-2 text-sm text-slate-700">
                  <input type="checkbox" checked={form.enabled} onChange={(e) => setForm({ ...form, enabled: e.target.checked })} />
                  Enabled
                </label>
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
