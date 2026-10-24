import { useState } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import { useTools } from '@/hooks/queries'
import PageHeader from '@/components/PageHeader'
import Loading from '@/components/Loading'
import ErrorState from '@/components/ErrorState'
import Card from '@/components/Card'
import type { Tool } from '@/types/domain'

const emptyForm = { id: '', description: '' }

export default function AdminToolsPage() {
  const { data: tools, isLoading, error } = useTools()
  const queryClient = useQueryClient()
  const [editingId, setEditingId] = useState<string | null>(null)
  const [form, setForm] = useState(emptyForm)
  const [message, setMessage] = useState<string | null>(null)

  const save = useMutation({
    mutationFn: (vars: { id: string; tool: Tool }) => api.saveTool(vars.id, vars.tool),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['tools'] })
      setEditingId(null)
      setMessage('Saved.')
    },
  })
  const remove = useMutation({
    mutationFn: (id: string) => api.deleteTool(id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['tools'] }),
  })

  function startNew() {
    setEditingId('__new__')
    setForm(emptyForm)
    setMessage(null)
  }

  function startEdit(t: Tool) {
    setEditingId(t.id)
    setForm({ id: t.id, description: t.description })
    setMessage(null)
  }

  function submit() {
    if (!editingId || !form.id) return
    const id = form.id
    save.mutate({ id, tool: { ...form, id } })
  }

  return (
    <div>
      <PageHeader title="Admin · Tools" subtitle="Available tools that agents can be granted." />
      <div className="mb-4">
        <button onClick={startNew} className="rounded-lg bg-slate-900 px-4 py-2 text-sm font-medium text-white hover:bg-slate-700">
          New tool
        </button>
      </div>

      {isLoading && <Loading />}
      {error && <ErrorState message={error.message} />}

      {tools && (
        <div className="grid gap-6 lg:grid-cols-3">
          <div className="lg:col-span-2 overflow-hidden rounded-xl border border-slate-200 bg-white shadow-sm">
            <table className="min-w-full divide-y divide-slate-200 text-sm">
              <thead className="bg-slate-50 text-left text-xs font-semibold uppercase tracking-wide text-slate-500">
                <tr>
                  <th className="px-4 py-3">Tool</th>
                  <th className="px-4 py-3">Description</th>
                  <th className="px-4 py-3"></th>
                </tr>
              </thead>
              <tbody className="divide-y divide-slate-100">
                {tools.map((t) => (
                  <tr key={t.id} className="hover:bg-slate-50">
                    <td className="px-4 py-3 font-mono text-sm text-slate-900">{t.id}</td>
                    <td className="px-4 py-3 text-slate-600">{t.description}</td>
                    <td className="px-4 py-3 text-right">
                      <button onClick={() => startEdit(t)} className="text-sm text-slate-600 hover:underline">Edit</button>
                      <button onClick={() => remove.mutate(t.id)} className="ml-3 text-sm text-red-600 hover:underline">Delete</button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>

          {editingId && (
            <Card>
              <h2 className="text-sm font-semibold text-slate-900">{editingId === '__new__' ? 'New tool' : `Editing ${editingId}`}</h2>
              <div className="mt-4 space-y-3">
                <Field label="ID (e.g. filesystem)">
                  <input value={form.id} disabled={editingId !== '__new__'} onChange={(e) => setForm({ ...form, id: e.target.value })} className={inputCls} />
                </Field>
                <Field label="Description">
                  <input value={form.description} onChange={(e) => setForm({ ...form, description: e.target.value })} className={inputCls} />
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
