import { useState } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import { useAgents, useWorkflows } from '@/hooks/queries'
import PageHeader from '@/components/PageHeader'
import Loading from '@/components/Loading'
import ErrorState from '@/components/ErrorState'
import Card from '@/components/Card'
import type { WorkflowDefinition, WorkflowStep } from '@/types/domain'

const emptySteps: WorkflowStep[] = [{ agentType: 'agent-codeact' }]

/** Derive a stable workflow id from its name (kebab-case), with a timestamp fallback. */
function slugify(name: string): string {
  const slug = name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
  if (slug) return slug
  return `workflow-${Date.now()}`
}

function duplicateError(steps: WorkflowStep[]): string | null {
  const seen = new Set<string>()
  for (const s of steps) {
    if (!s.agentType) continue
    if (seen.has(s.agentType)) return `agent type '${s.agentType}' appears more than once`
    seen.add(s.agentType)
  }
  return null
}

export default function AdminWorkflowsPage() {
  const { data: workflows, isLoading, error } = useWorkflows()
  const { data: agents } = useAgents()
  const queryClient = useQueryClient()
  const [editingId, setEditingId] = useState<string | null>(null)
  const [form, setForm] = useState({ name: '', isDefault: false, steps: emptySteps })
  const [message, setMessage] = useState<string | null>(null)
  const [errorMsg, setErrorMsg] = useState<string | null>(null)

  const save = useMutation({
    mutationFn: (wf: WorkflowDefinition) => api.saveWorkflow(wf.id, wf),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['workflows'] })
      setEditingId(null)
      setMessage('Saved.')
      setErrorMsg(null)
    },
    onError: (err: Error) => {
      setMessage(null)
      setErrorMsg(err.message)
    },
  })
  const remove = useMutation({
    mutationFn: (id: string) => api.deleteWorkflow(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['workflows'] })
      setEditingId(null)
    },
  })

  const dup = duplicateError(form.steps)

  function startNew() {
    setEditingId('__new__')
    setForm({ name: '', isDefault: false, steps: [{ agentType: 'agent-codeact' }] })
    setMessage(null)
    setErrorMsg(null)
  }

  function startEdit(wf: WorkflowDefinition) {
    setEditingId(wf.id)
    setForm({
      name: wf.name,
      isDefault: wf.isDefault,
      steps: wf.steps.map((s) => ({ ...s })),
    })
    setMessage(null)
    setErrorMsg(null)
  }

  function submit() {
    if (!editingId) return
    if (form.steps.length === 0) {
      setErrorMsg('Add at least one step.')
      return
    }
    if (form.steps.some((s) => !s.agentType)) {
      setErrorMsg('Every step needs an agent type.')
      return
    }
    if (dup) {
      setErrorMsg(dup)
      return
    }
    const id = editingId === '__new__' ? slugify(form.name) : editingId
    save.mutate({ id, name: form.name, isDefault: form.isDefault, steps: form.steps })
  }

  function setStep(i: number, agentType: string) {
    setForm({ ...form, steps: form.steps.map((s, idx) => (idx === i ? { ...s, agentType } : s)) })
  }
  function addStep() {
    setForm({ ...form, steps: [...form.steps, { agentType: 'agent-codeact' }] })
  }
  function removeStep(i: number) {
    setForm({ ...form, steps: form.steps.filter((_, idx) => idx !== i) })
  }
  function moveStep(i: number, dir: -1 | 1) {
    const j = i + dir
    if (j < 0 || j >= form.steps.length) return
    const steps = [...form.steps]
    const a = steps[i]
    const b = steps[j]
    if (a === undefined || b === undefined) return
    steps[i] = b
    steps[j] = a
    setForm({ ...form, steps })
  }

  return (
    <div>
      <PageHeader
        title="Admin · Workflows"
        subtitle="Ordered agent pipelines. A workflow run compiles the steps into a LangGraph StateGraph — coding, then documentation, then review by default."
      />
      <div className="mb-4">
        <button
          onClick={startNew}
          className="rounded-lg bg-slate-900 px-4 py-2 text-sm font-medium text-white hover:bg-slate-700"
        >
          New workflow
        </button>
      </div>

      {errorMsg && <p className="mb-4 text-sm text-red-600">{errorMsg}</p>}
      {message && <p className="mb-4 text-sm text-emerald-600">{message}</p>}

      {isLoading && <Loading />}
      {error && <ErrorState message={error.message} />}

      {workflows && (
        <div className="grid gap-6 lg:grid-cols-3">
          <div className="lg:col-span-2 overflow-hidden rounded-xl border border-slate-200 bg-white shadow-sm">
            <table className="min-w-full divide-y divide-slate-200 text-sm">
              <thead className="bg-slate-50 text-left text-xs font-semibold uppercase tracking-wide text-slate-500">
                <tr>
                  <th className="px-4 py-3">Workflow</th>
                  <th className="px-4 py-3">Pipeline</th>
                  <th className="px-4 py-3"></th>
                </tr>
              </thead>
              <tbody className="divide-y divide-slate-100">
                {workflows.map((w) => (
                  <tr key={w.id} className="hover:bg-slate-50">
                    <td className="px-4 py-3">
                      <div className="font-medium text-slate-900">
                        {w.name}
                        {w.isDefault && (
                          <span className="ml-2 rounded bg-slate-100 px-1.5 py-0.5 text-xs text-slate-500">
                            default
                          </span>
                        )}
                      </div>
                      <div className="text-xs text-slate-400">{w.id}</div>
                    </td>
                    <td className="px-4 py-3 font-mono text-xs text-slate-600">
                      {w.steps.map((s) => s.agentType).join(' → ')}
                    </td>
                    <td className="px-4 py-3 text-right">
                      <button onClick={() => startEdit(w)} className="text-sm text-slate-600 hover:underline">
                        Edit
                      </button>
                      <button
                        onClick={() => remove.mutate(w.id)}
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
              <h2 className="text-sm font-semibold text-slate-900">
                {editingId === '__new__' ? 'New workflow' : `Editing ${form.name || editingId}`}
              </h2>
              <div className="mt-4 space-y-3">
                <Field label="Name">
                  <input
                    value={form.name}
                    onChange={(e) => setForm({ ...form, name: e.target.value })}
                    placeholder="Release pipeline"
                    className={inputCls}
                  />
                </Field>
                <label className="flex items-center gap-2 text-sm text-slate-700">
                  <input
                    type="checkbox"
                    checked={form.isDefault}
                    onChange={(e) => setForm({ ...form, isDefault: e.target.checked })}
                  />
                  Default workflow
                </label>

                <div>
                  <span className="mb-1 block text-sm font-medium text-slate-700">Steps</span>
                  <div className="space-y-2">
                    {form.steps.map((s, i) => (
                      <div key={i} className="flex items-center gap-2 rounded-lg border border-slate-200 p-2">
                        <span className="w-5 shrink-0 text-center text-xs font-semibold text-slate-400">
                          {i + 1}
                        </span>
                        <select
                          value={s.agentType}
                          onChange={(e) => setStep(i, e.target.value)}
                          className={`${inputCls} min-w-0 flex-1`}
                        >
                          {(agents ?? []).map((a) => (
                            <option key={a.id} value={a.id}>
                              {a.name} ({a.id})
                            </option>
                          ))}
                        </select>
                        <button
                          onClick={() => moveStep(i, -1)}
                          disabled={i === 0}
                          className="rounded px-1.5 py-1 text-slate-500 hover:bg-slate-100 disabled:opacity-30"
                          title="Move up"
                        >
                          ↑
                        </button>
                        <button
                          onClick={() => moveStep(i, 1)}
                          disabled={i === form.steps.length - 1}
                          className="rounded px-1.5 py-1 text-slate-500 hover:bg-slate-100 disabled:opacity-30"
                          title="Move down"
                        >
                          ↓
                        </button>
                        <button
                          onClick={() => removeStep(i)}
                          disabled={form.steps.length === 1}
                          className="rounded px-1.5 py-1 text-red-500 hover:bg-red-50 disabled:opacity-30"
                          title="Remove step"
                        >
                          ✕
                        </button>
                      </div>
                    ))}
                  </div>
                  <button
                    onClick={addStep}
                    className="mt-2 text-sm text-blue-600 hover:underline"
                  >
                    + Add step
                  </button>
                </div>

                {dup && <p className="text-sm text-amber-600">{dup}</p>}

                <div className="flex gap-2 pt-1">
                  <button
                    onClick={submit}
                    disabled={save.isPending}
                    className="rounded-lg bg-slate-900 px-4 py-2 text-sm font-medium text-white hover:bg-slate-700 disabled:opacity-50"
                  >
                    {save.isPending ? 'Saving…' : 'Save'}
                  </button>
                  <button
                    onClick={() => setEditingId(null)}
                    className="rounded-lg px-4 py-2 text-sm text-slate-600 hover:bg-slate-100"
                  >
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
