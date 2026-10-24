import { useState } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import { useGitHosts, useRepos } from '@/hooks/queries'
import PageHeader from '@/components/PageHeader'
import Loading from '@/components/Loading'
import ErrorState from '@/components/ErrorState'
import Card from '@/components/Card'
import type { GitRepo, GitRepoWrite } from '@/types/domain'

const emptyForm = {
  host: 'github.com',
  repo: '',
  enabled: true,
  token: '',
  writeToken: '',
}

export default function AdminReposPage() {
  const { data: repos, isLoading, error } = useRepos()
  const { data: gitHosts } = useGitHosts()
  const queryClient = useQueryClient()
  const [editingId, setEditingId] = useState<string | null>(null)
  const [form, setForm] = useState(emptyForm)
  const [message, setMessage] = useState<string | null>(null)
  const [errorMsg, setErrorMsg] = useState<string | null>(null)

  const hosts = gitHosts && gitHosts.length > 0 ? gitHosts : [{ host: 'github.com', label: 'GitHub' }]

  const save = useMutation({
    mutationFn: (write: GitRepoWrite) => api.saveRepo(write),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['repos'] })
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
    mutationFn: (id: string) => api.deleteRepo(id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['repos'] }),
  })
  const rotate = useMutation({
    mutationFn: () => api.rotateEncryptionKey(),
    onSuccess: () => {
      setMessage('Encryption key rotated.')
      setErrorMsg(null)
    },
    onError: (err: Error) => {
      setMessage(null)
      setErrorMsg(err.message)
    },
  })

  function onRotate() {
    if (
      window.confirm(
        'Rotate the encryption key? All stored git tokens will be re-encrypted under a new key.',
      )
    ) {
      rotate.mutate()
    }
  }

  function startNew() {
    setEditingId('__new__')
    setForm(emptyForm)
    setMessage(null)
    setErrorMsg(null)
  }

  function startEdit(r: GitRepo) {
    setEditingId(r.id)
    setForm({
      host: r.host,
      repo: r.name,
      enabled: r.enabled,
      token: '',
      writeToken: '',
    })
    setMessage(null)
    setErrorMsg(null)
  }

  function submit() {
    if (!form.repo.trim()) return
    const payload: GitRepoWrite = {
      host: form.host,
      repo: form.repo.trim(),
      enabled: form.enabled,
      ...(form.token ? { token: form.token } : {}),
      ...(form.writeToken ? { writeToken: form.writeToken } : {}),
    }
    save.mutate(payload)
  }

  return (
    <div>
      <PageHeader
        title="Admin · Repos"
        subtitle="Git repositories agents can clone, with per-repo access tokens (encrypted at rest)."
      />
      <div className="mb-4 flex items-center gap-3">
        <button
          onClick={startNew}
          className="rounded-lg bg-slate-900 px-4 py-2 text-sm font-medium text-white hover:bg-slate-700"
        >
          New repo
        </button>
        <button
          onClick={onRotate}
          disabled={rotate.isPending}
          className="rounded-lg border border-slate-300 px-4 py-2 text-sm font-medium text-slate-700 hover:bg-slate-100 disabled:opacity-50"
        >
          {rotate.isPending ? 'Rotating…' : 'Rotate encryption key'}
        </button>
      </div>

      {errorMsg && <p className="mb-4 text-sm text-red-600">{errorMsg}</p>}
      {message && <p className="mb-4 text-sm text-emerald-600">{message}</p>}

      {isLoading && <Loading />}
      {error && <ErrorState message={error.message} />}

      {repos && (
        <div className="grid gap-6 lg:grid-cols-3">
          <div className="lg:col-span-2 overflow-hidden rounded-xl border border-slate-200 bg-white shadow-sm">
            <table className="min-w-full divide-y divide-slate-200 text-sm">
              <thead className="bg-slate-50 text-left text-xs font-semibold uppercase tracking-wide text-slate-500">
                <tr>
                  <th className="px-4 py-3">Repo</th>
                  <th className="px-4 py-3">Token</th>
                  <th className="px-4 py-3"></th>
                </tr>
              </thead>
              <tbody className="divide-y divide-slate-100">
                {repos.map((r) => (
                  <tr key={r.id} className="hover:bg-slate-50">
                    <td className="px-4 py-3">
                      <div className="font-medium text-slate-900">
                        <span className="mr-1.5 rounded bg-slate-100 px-1.5 py-0.5 text-xs text-slate-500">
                          {r.host}
                        </span>
                        {r.name}
                      </div>
                      <div className="font-mono text-xs text-slate-400">{r.url}</div>
                    </td>
                    <td className="px-4 py-3 text-slate-600">
                      {r.hasToken ? '••••••••' : '—'}
                      {!r.enabled && <span className="ml-2 text-xs text-slate-400">(disabled)</span>}
                    </td>
                    <td className="px-4 py-3 text-right">
                      <button onClick={() => startEdit(r)} className="text-sm text-slate-600 hover:underline">
                        Edit
                      </button>
                      <button onClick={() => remove.mutate(r.id)} className="ml-3 text-sm text-red-600 hover:underline">
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
                {editingId === '__new__' ? 'New repo' : `Editing ${form.repo || editingId}`}
              </h2>
              <div className="mt-4 space-y-3">
                <Field label="Host">
                  <select
                    value={form.host}
                    onChange={(e) => setForm({ ...form, host: e.target.value })}
                    className={inputCls}
                  >
                    {hosts.map((h) => (
                      <option key={h.host} value={h.host}>
                        {h.label} ({h.host})
                      </option>
                    ))}
                  </select>
                </Field>
                <Field label="Repository path">
                  <input
                    value={form.repo}
                    onChange={(e) => setForm({ ...form, repo: e.target.value })}
                    placeholder="org/repo"
                    className={inputCls}
                  />
                  <span className="mt-1 block text-xs text-slate-400">
                    e.g. <code className="font-mono">octocat/hello-world</code> — the URL and
                    identifier are derived for you.
                  </span>
                </Field>
                <Field label="Access token">
                  <input
                    type="password"
                    value={form.token}
                    onChange={(e) => setForm({ ...form, token: e.target.value })}
                    placeholder={editingId === '__new__' ? 'ghp_…' : 'Leave blank to keep existing token'}
                    className={inputCls}
                  />
                  <span className="mt-1 block text-xs text-slate-400">
                    Read token (for cloning). Encrypted at rest with the control plane's
                    encryption key. Never returned by the API.
                  </span>
                </Field>
                <Field label="Write token">
                  <input
                    type="password"
                    value={form.writeToken}
                    onChange={(e) => setForm({ ...form, writeToken: e.target.value })}
                    placeholder={editingId === '__new__' ? 'ghp_…' : 'Leave blank to keep existing token'}
                    className={inputCls}
                  />
                  <span className="mt-1 block text-xs text-slate-400">
                    Used to open PRs/MRs from the agent's output. Encrypted at rest like the
                    read token. Never returned by the API.
                  </span>
                </Field>
                <label className="flex items-center gap-2 text-sm text-slate-700">
                  <input
                    type="checkbox"
                    checked={form.enabled}
                    onChange={(e) => setForm({ ...form, enabled: e.target.checked })}
                  />
                  Enabled
                </label>
                <div className="flex gap-2 pt-1">
                  <button
                    onClick={submit}
                    disabled={save.isPending}
                    className="rounded-lg bg-slate-900 px-4 py-2 text-sm font-medium text-white hover:bg-slate-700"
                  >
                    Save
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
