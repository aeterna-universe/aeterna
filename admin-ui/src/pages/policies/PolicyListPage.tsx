import { useState } from "react"
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query"
import { ScrollText, Loader2, Plus, X } from "lucide-react"
import { cn } from "@/lib/utils"
import { apiClient } from "@/api/client"
import type { PolicyRecord } from "@/api/types"

function CreatePolicyDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const queryClient = useQueryClient()
  const [name, setName] = useState("")
  const [description, setDescription] = useState("")
  const [layer, setLayer] = useState("tenant")
  const [mode, setMode] = useState("mandatory")

  const create = useMutation({
    mutationFn: (data: { name: string; description: string; layer: string; mode: string }) =>
      apiClient.post("/api/v1/govern/policies", data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["policies"] })
      setName("")
      setDescription("")
      onClose()
    },
  })

  if (!open) return null

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
      <div className="w-full max-w-md rounded-lg bg-white p-6 shadow-xl dark:bg-gray-800">
        <div className="mb-4 flex items-center justify-between">
          <h2 className="text-lg font-semibold text-gray-900 dark:text-gray-100">Create Policy</h2>
          <button onClick={onClose} className="text-gray-400 hover:text-gray-600">
            <X className="h-5 w-5" />
          </button>
        </div>
        <form
          onSubmit={(e) => {
            e.preventDefault()
            create.mutate({ name, description, layer, mode })
          }}
        >
          <div className="space-y-4">
            <div>
              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300">Name</label>
              <input
                type="text"
                value={name}
                onChange={(e) => setName(e.target.value)}
                required
                className="mt-1 block w-full rounded-md border border-gray-300 px-3 py-2 text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-700 dark:text-gray-100"
              />
            </div>
            <div>
              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300">
                Description
              </label>
              <textarea
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                rows={2}
                className="mt-1 block w-full rounded-md border border-gray-300 px-3 py-2 text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-700 dark:text-gray-100"
              />
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300">
                  Layer
                </label>
                <select
                  value={layer}
                  onChange={(e) => setLayer(e.target.value)}
                  className="mt-1 block w-full rounded-md border border-gray-300 px-3 py-2 text-sm dark:border-gray-600 dark:bg-gray-700 dark:text-gray-100"
                >
                  <option value="tenant">Tenant</option>
                  <option value="Organization">Organization</option>
                  <option value="Team">Team</option>
                  <option value="Project">Project</option>
                </select>
              </div>
              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300">
                  Mode
                </label>
                <select
                  value={mode}
                  onChange={(e) => setMode(e.target.value)}
                  className="mt-1 block w-full rounded-md border border-gray-300 px-3 py-2 text-sm dark:border-gray-600 dark:bg-gray-700 dark:text-gray-100"
                >
                  <option value="mandatory">Mandatory</option>
                  <option value="optional">Optional</option>
                </select>
              </div>
            </div>
          </div>
          {create.isError && (
            <p className="mt-2 text-sm text-red-600">Failed to create policy.</p>
          )}
          <div className="mt-6 flex justify-end gap-3">
            <button
              type="button"
              onClick={onClose}
              className="rounded-md border border-gray-300 px-4 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50 dark:border-gray-600 dark:text-gray-300"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={create.isPending}
              className="inline-flex items-center gap-2 rounded-md bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:opacity-50"
            >
              {create.isPending && <Loader2 className="h-4 w-4 animate-spin" />}
              Create
            </button>
          </div>
        </form>
      </div>
    </div>
  )
}

export function Component() {
  return <PolicyListPage />
}

export default function PolicyListPage() {
  const [dialogOpen, setDialogOpen] = useState(false)

  const { data, isLoading, error, refetch } = useQuery<PolicyRecord[] | { items: PolicyRecord[] }>({
    queryKey: ["policies"],
    queryFn: () => apiClient.get("/api/v1/govern/policies"),
  })

  const policies: PolicyRecord[] = Array.isArray(data) ? data : (data?.items ?? [])

  const modeColor: Record<string, string> = {
    mandatory: "bg-red-100 text-red-700 dark:bg-red-900 dark:text-red-300",
    optional: "bg-gray-100 text-gray-700 dark:bg-gray-700 dark:text-gray-300",
  }

  return (
    <div>
      <div className="mb-6 flex items-center justify-between">
        <div className="flex items-center gap-3">
          <ScrollText className="h-6 w-6 text-gray-400" />
          <h1 className="text-2xl font-semibold text-gray-900 dark:text-gray-100">Policies</h1>
        </div>
        <button
          onClick={() => setDialogOpen(true)}
          className="inline-flex items-center gap-2 rounded-md bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700"
        >
          <Plus className="h-4 w-4" />
          Create Policy
        </button>
      </div>

      {isLoading && (
        <div className="flex justify-center p-8">
          <Loader2 className="h-6 w-6 animate-spin text-gray-400" />
        </div>
      )}

      {error && (
        <div className="p-8 text-center text-red-600">
          Failed to load policies.{" "}
          <button onClick={() => refetch()} className="underline">
            Retry
          </button>
        </div>
      )}

      {!isLoading && !error && (
        <div className="overflow-hidden rounded-lg border border-gray-200 dark:border-gray-700">
          <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
            <thead className="bg-gray-50 dark:bg-gray-800">
              <tr>
                <th className="px-4 py-3 text-left text-xs font-medium uppercase tracking-wider text-gray-500">
                  Name
                </th>
                <th className="px-4 py-3 text-left text-xs font-medium uppercase tracking-wider text-gray-500">
                  Layer
                </th>
                <th className="px-4 py-3 text-left text-xs font-medium uppercase tracking-wider text-gray-500">
                  Mode
                </th>
                <th className="px-4 py-3 text-left text-xs font-medium uppercase tracking-wider text-gray-500">
                  Rules
                </th>
              </tr>
            </thead>
            <tbody className="divide-y divide-gray-200 bg-white dark:divide-gray-700 dark:bg-gray-900">
              {policies.length === 0 ? (
                <tr>
                  <td colSpan={4} className="px-4 py-8 text-center text-sm text-gray-500">
                    No policies found.
                  </td>
                </tr>
              ) : (
                policies.map((policy) => (
                  <tr key={policy.id} className="hover:bg-gray-50 dark:hover:bg-gray-800">
                    <td className="px-4 py-3">
                      <div className="text-sm font-medium text-gray-900 dark:text-gray-100">
                        {policy.name}
                      </div>
                      {policy.description && (
                        <div className="text-xs text-gray-500 dark:text-gray-400">
                          {policy.description}
                        </div>
                      )}
                    </td>
                    <td className="whitespace-nowrap px-4 py-3 text-sm text-gray-500 dark:text-gray-400">
                      {policy.layer}
                    </td>
                    <td className="whitespace-nowrap px-4 py-3 text-sm">
                      <span
                        className={cn(
                          "inline-flex rounded-full px-2 py-0.5 text-xs font-medium",
                          modeColor[policy.mode.toLowerCase()] ?? "bg-gray-100 text-gray-700",
                        )}
                      >
                        {policy.mode}
                      </span>
                    </td>
                    <td className="whitespace-nowrap px-4 py-3 text-sm text-gray-500 dark:text-gray-400">
                      {Object.keys(policy.rules).length}
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      )}

      <CreatePolicyDialog open={dialogOpen} onClose={() => setDialogOpen(false)} />
    </div>
  )
}
