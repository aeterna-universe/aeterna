import { useState } from "react"
import { useNavigate } from "react-router-dom"
import { useQuery } from "@tanstack/react-query"
import { BookOpen, Search, Loader2, Filter } from "lucide-react"
import { cn } from "@/lib/utils"
import { apiClient } from "@/api/client"
import type { KnowledgeEntry, KnowledgeType, KnowledgeLayer } from "@/api/types"

const KNOWLEDGE_TYPES: KnowledgeType[] = ["Adr", "Policy", "Pattern", "Spec", "Hindsight"]
const KNOWLEDGE_LAYERS: KnowledgeLayer[] = ["Tenant", "Organization", "Team", "Project"]

const typeBadgeColor: Record<string, string> = {
  Adr: "bg-blue-100 text-blue-700 dark:bg-blue-900 dark:text-blue-300",
  Policy: "bg-purple-100 text-purple-700 dark:bg-purple-900 dark:text-purple-300",
  Pattern: "bg-teal-100 text-teal-700 dark:bg-teal-900 dark:text-teal-300",
  Spec: "bg-orange-100 text-orange-700 dark:bg-orange-900 dark:text-orange-300",
  Hindsight: "bg-pink-100 text-pink-700 dark:bg-pink-900 dark:text-pink-300",
}

const layerBadgeColor: Record<string, string> = {
  Tenant: "bg-indigo-100 text-indigo-700 dark:bg-indigo-900 dark:text-indigo-300",
  Organization: "bg-cyan-100 text-cyan-700 dark:bg-cyan-900 dark:text-cyan-300",
  Team: "bg-emerald-100 text-emerald-700 dark:bg-emerald-900 dark:text-emerald-300",
  Project: "bg-amber-100 text-amber-700 dark:bg-amber-900 dark:text-amber-300",
}

export function Component() {
  return <KnowledgeSearchPage />
}

export default function KnowledgeSearchPage() {
  const navigate = useNavigate()
  const [query, setQuery] = useState("")
  const [submittedQuery, setSubmittedQuery] = useState("")
  const [typeFilter, setTypeFilter] = useState<KnowledgeType | "">("")
  const [layerFilter, setLayerFilter] = useState<KnowledgeLayer | "">("")

  const { data, isLoading, error, refetch } = useQuery<{
    items: Array<KnowledgeEntry & { score?: number; snippet?: string }>
  }>({
    queryKey: ["knowledge", "search", submittedQuery, typeFilter, layerFilter],
    queryFn: () =>
      apiClient.post("/api/v1/knowledge/query", {
        query: submittedQuery,
        ...(typeFilter && { kind: typeFilter }),
        ...(layerFilter && { layer: layerFilter }),
      }),
    enabled: submittedQuery.length > 0,
  })

  const results = data?.items ?? []

  const handleSearch = (e: React.FormEvent) => {
    e.preventDefault()
    setSubmittedQuery(query)
  }

  return (
    <div>
      <div className="mb-6 flex items-center gap-3">
        <BookOpen className="h-6 w-6 text-gray-400" />
        <h1 className="text-2xl font-semibold text-gray-900 dark:text-gray-100">Knowledge</h1>
      </div>

      <form onSubmit={handleSearch} className="mb-6 space-y-3">
        <div className="flex gap-3">
          <div className="relative flex-1">
            <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-gray-400" />
            <input
              type="text"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search knowledge entries..."
              className="w-full rounded-md border border-gray-300 py-2 pl-9 pr-3 text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-700 dark:text-gray-100"
            />
          </div>
          <button
            type="submit"
            className="rounded-md bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700"
          >
            Search
          </button>
        </div>
        <div className="flex items-center gap-3">
          <Filter className="h-4 w-4 text-gray-400" />
          <select
            value={typeFilter}
            onChange={(e) => setTypeFilter(e.target.value as KnowledgeType | "")}
            className="rounded-md border border-gray-300 px-3 py-1.5 text-sm dark:border-gray-600 dark:bg-gray-700 dark:text-gray-100"
          >
            <option value="">All types</option>
            {KNOWLEDGE_TYPES.map((t) => (
              <option key={t} value={t}>
                {t}
              </option>
            ))}
          </select>
          <select
            value={layerFilter}
            onChange={(e) => setLayerFilter(e.target.value as KnowledgeLayer | "")}
            className="rounded-md border border-gray-300 px-3 py-1.5 text-sm dark:border-gray-600 dark:bg-gray-700 dark:text-gray-100"
          >
            <option value="">All layers</option>
            {KNOWLEDGE_LAYERS.map((l) => (
              <option key={l} value={l}>
                {l}
              </option>
            ))}
          </select>
        </div>
      </form>

      {!submittedQuery && (
        <div className="rounded-lg border border-dashed border-gray-300 p-12 text-center dark:border-gray-600">
          <BookOpen className="mx-auto h-12 w-12 text-gray-300 dark:text-gray-600" />
          <p className="mt-3 text-sm text-gray-500">Enter a query to search knowledge entries.</p>
        </div>
      )}

      {isLoading && (
        <div className="flex justify-center p-8">
          <Loader2 className="h-6 w-6 animate-spin text-gray-400" />
        </div>
      )}

      {error && (
        <div className="p-8 text-center text-red-600">
          Search failed.{" "}
          <button onClick={() => refetch()} className="underline">
            Retry
          </button>
        </div>
      )}

      {submittedQuery && !isLoading && !error && (
        <div className="space-y-3">
          {results.length === 0 ? (
            <p className="py-8 text-center text-sm text-gray-500">No results found.</p>
          ) : (
            results.map((entry) => (
              <div
                key={entry.id}
                onClick={() => navigate(`/admin/knowledge/${entry.id}`)}
                className="cursor-pointer rounded-lg border border-gray-200 bg-white p-4 hover:border-blue-300 hover:shadow-sm dark:border-gray-700 dark:bg-gray-800 dark:hover:border-blue-600"
              >
                <div className="flex items-start justify-between">
                  <div className="min-w-0 flex-1">
                    <h3 className="truncate font-medium text-gray-900 dark:text-gray-100">
                      {entry.path || entry.id}
                    </h3>
                    {entry.snippet && (
                      <p className="mt-1 line-clamp-2 text-sm text-gray-500 dark:text-gray-400">
                        {entry.snippet}
                      </p>
                    )}
                  </div>
                  {entry.score !== undefined && (
                    <span className="ml-3 shrink-0 text-xs text-gray-400">
                      {(entry.score * 100).toFixed(0)}%
                    </span>
                  )}
                </div>
                <div className="mt-2 flex flex-wrap gap-2">
                  <span
                    className={cn(
                      "inline-flex rounded-full px-2 py-0.5 text-xs font-medium",
                      typeBadgeColor[entry.kind] ?? "bg-gray-100 text-gray-700",
                    )}
                  >
                    {entry.kind}
                  </span>
                  <span
                    className={cn(
                      "inline-flex rounded-full px-2 py-0.5 text-xs font-medium",
                      layerBadgeColor[entry.layer] ?? "bg-gray-100 text-gray-700",
                    )}
                  >
                    {entry.layer}
                  </span>
                  <span className="inline-flex rounded-full bg-gray-100 px-2 py-0.5 text-xs font-medium text-gray-600 dark:bg-gray-700 dark:text-gray-400">
                    {entry.status}
                  </span>
                </div>
              </div>
            ))
          )}
        </div>
      )}
    </div>
  )
}
