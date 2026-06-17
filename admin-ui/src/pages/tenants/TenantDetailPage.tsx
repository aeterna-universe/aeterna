import { useState } from "react";
import { useParams, useNavigate } from "react-router-dom";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Building2, Loader2, ArrowLeft, Save, Download } from "lucide-react";
import { cn } from "@/lib/utils";
import { apiClient } from "@/api/client";
import type { TenantRecord } from "@/api/types";
import { manifestToYaml } from "@/api/tenant-manifest";
import { getStoredTokens } from "@/auth/token-manager";

/**
 * §12.9 — Download manifest. Calls `GET /api/v1/admin/tenants/{slug}/manifest`
 * with `?redact=true` (operator-friendly default), renders YAML in a `<pre>`,
 * and offers a download as `<slug>.manifest.yaml`. Plaintext download is
 * gated by an explicit toggle so accidental clicks never leak references.
 */
function useDownloadManifest(tenantSlug: string) {
	const [redacted, setRedacted] = useState(true);
	const [yaml, setYaml] = useState<string | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [isLoading, setLoading] = useState(false);

	const fetchManifest = async (redact: boolean) => {
		setLoading(true);
		setError(null);
		try {
			const tokens = getStoredTokens();
			const headers: Record<string, string> = {
				"X-Aeterna-Client-Kind": "ui",
			};
			if (tokens) headers["Authorization"] = `Bearer ${tokens.access_token}`;
			const url = `/api/v1/admin/tenants/${encodeURIComponent(tenantSlug)}/manifest${
				redact ? "?redact=true" : ""
			}`;
			const res = await fetch(url, { headers });
			if (!res.ok) {
				throw new Error(`HTTP ${res.status}`);
			}
			const json = await res.json();
			setYaml(manifestToYaml(json));
			setRedacted(redact);
		} catch (e) {
			setError((e as Error).message);
		} finally {
			setLoading(false);
		}
	};

	const download = () => {
		if (!yaml) return;
		const blob = new Blob([yaml], { type: "application/yaml" });
		const url = URL.createObjectURL(blob);
		const a = document.createElement("a");
		a.href = url;
		a.download = `${tenantSlug}.manifest${redacted ? ".redacted" : ""}.yaml`;
		document.body.appendChild(a);
		a.click();
		a.remove();
		URL.revokeObjectURL(url);
	};

	return {
		yaml,
		error,
		isLoading,
		redacted,
		fetchManifest,
		download,
		reset: () => setYaml(null),
	};
}

function ManifestPanel({ tenantSlug }: { tenantSlug: string }) {
	const dl = useDownloadManifest(tenantSlug);
	return (
		<div className="space-y-3" data-testid="manifest-panel">
			<div className="flex flex-wrap items-center gap-2">
				<button
					type="button"
					onClick={() => dl.fetchManifest(true)}
					disabled={dl.isLoading}
					className="inline-flex items-center gap-2 rounded-md border border-gray-300 px-3 py-1.5 text-sm hover:bg-gray-50 disabled:opacity-50 dark:border-gray-600 dark:hover:bg-gray-700"
					data-testid="manifest-fetch-redacted"
				>
					{dl.isLoading && <Loader2 className="h-4 w-4 animate-spin" />}
					Render manifest (redacted)
				</button>
				<button
					type="button"
					onClick={() => dl.fetchManifest(false)}
					disabled={dl.isLoading}
					className="inline-flex items-center gap-2 rounded-md border border-amber-300 bg-amber-50 px-3 py-1.5 text-sm text-amber-900 hover:bg-amber-100 disabled:opacity-50 dark:border-amber-700 dark:bg-amber-900/30 dark:text-amber-200"
					data-testid="manifest-fetch-plain"
				>
					Render with secret references
				</button>
				{dl.yaml && (
					<button
						type="button"
						onClick={dl.download}
						className="inline-flex items-center gap-2 rounded-md bg-blue-600 px-3 py-1.5 text-sm text-white hover:bg-blue-700"
						data-testid="manifest-download"
					>
						<Download className="h-4 w-4" /> Download YAML
					</button>
				)}
			</div>
			{dl.error && <p className="text-sm text-red-600">Failed: {dl.error}</p>}
			{dl.yaml && (
				<pre
					className="max-h-[60vh] overflow-auto rounded-md bg-gray-900 p-4 font-mono text-xs text-gray-100"
					data-testid="manifest-yaml"
				>
					{dl.yaml}
				</pre>
			)}
		</div>
	);
}

type Tab = "overview" | "config" | "providers" | "repository" | "manifest";

function OverviewTab({ tenant }: { tenant: TenantRecord }) {
	const navigate = useNavigate();
	const queryClient = useQueryClient();

	const deactivate = useMutation({
		mutationFn: () =>
			apiClient.patch(`/api/v1/admin/tenants/${tenant.slug}`, {
				status: "inactive",
			}),
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: ["tenant", tenant.slug] });
		},
	});

	return (
		<div className="space-y-6">
			<div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
				<div>
					<dt className="text-sm font-medium text-gray-500 dark:text-gray-400">
						Name
					</dt>
					<dd className="mt-1 text-sm text-gray-900 dark:text-gray-100">
						{tenant.name}
					</dd>
				</div>
				<div>
					<dt className="text-sm font-medium text-gray-500 dark:text-gray-400">
						Slug
					</dt>
					<dd className="mt-1 font-mono text-sm text-gray-900 dark:text-gray-100">
						{tenant.slug}
					</dd>
				</div>
				<div>
					<dt className="text-sm font-medium text-gray-500 dark:text-gray-400">
						Account
					</dt>
					<dd className="mt-1 text-sm text-gray-900 dark:text-gray-100">
						{tenant.account
							? `${tenant.account.name} (${tenant.account.slug})`
							: "—"}
					</dd>
				</div>
				<div>
					<dt className="text-sm font-medium text-gray-500 dark:text-gray-400">
						Environment
					</dt>
					<dd className="mt-1 text-sm text-gray-900 dark:text-gray-100">
						{tenant.environment ?? "—"}
					</dd>
				</div>
				<div>
					<dt className="text-sm font-medium text-gray-500 dark:text-gray-400">
						Status
					</dt>
					<dd className="mt-1">
						<span
							className={cn(
								"inline-flex rounded-full px-2 py-0.5 text-xs font-medium",
								tenant.status?.toLowerCase() === "active"
									? "bg-green-100 text-green-700 dark:bg-green-900 dark:text-green-300"
									: "bg-gray-100 text-gray-700 dark:bg-gray-700 dark:text-gray-300",
							)}
						>
							{tenant.status}
						</span>
					</dd>
				</div>
				<div>
					<dt className="text-sm font-medium text-gray-500 dark:text-gray-400">
						Created
					</dt>
					<dd className="mt-1 text-sm text-gray-900 dark:text-gray-100">
						{new Date(tenant.createdAt).toLocaleString()}
					</dd>
				</div>
			</div>
			<div className="flex gap-3">
				{tenant.status?.toLowerCase() === "active" && (
					<button
						onClick={() => deactivate.mutate()}
						disabled={deactivate.isPending}
						className="inline-flex items-center gap-2 rounded-md border border-red-300 px-4 py-2 text-sm font-medium text-red-700 hover:bg-red-50 disabled:opacity-50 dark:border-red-700 dark:text-red-400 dark:hover:bg-red-900/20"
					>
						{deactivate.isPending && (
							<Loader2 className="h-4 w-4 animate-spin" />
						)}
						Deactivate Tenant
					</button>
				)}
				<button
					onClick={() => navigate("/admin/tenants")}
					className="rounded-md border border-gray-300 px-4 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50 dark:border-gray-600 dark:text-gray-300 dark:hover:bg-gray-700"
				>
					Back to List
				</button>
			</div>
		</div>
	);
}

function ConfigTab({ tenantSlug }: { tenantSlug: string }) {
	const queryClient = useQueryClient();
	const [editingKey, setEditingKey] = useState<string | null>(null);
	const [editValue, setEditValue] = useState("");

	const { data, isLoading, error, refetch } = useQuery<Record<string, string>>({
		queryKey: ["tenant", tenantSlug, "config"],
		queryFn: () => apiClient.get(`/api/v1/admin/tenants/${tenantSlug}/config`),
	});

	const updateConfig = useMutation({
		mutationFn: ({ key, value }: { key: string; value: string }) =>
			apiClient.patch(`/api/v1/admin/tenants/${tenantSlug}/config`, {
				[key]: value,
			}),
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["tenant", tenantSlug, "config"],
			});
			setEditingKey(null);
		},
	});

	if (isLoading) {
		return (
			<div className="flex justify-center p-8">
				<Loader2 className="h-6 w-6 animate-spin text-gray-400" />
			</div>
		);
	}

	if (error) {
		return (
			<div className="p-8 text-center text-red-600">
				Failed to load config.{" "}
				<button onClick={() => refetch()} className="underline">
					Retry
				</button>
			</div>
		);
	}

	const entries = Object.entries(data ?? {});

	return (
		<div className="overflow-hidden rounded-lg border border-gray-200 dark:border-gray-700">
			<table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
				<thead className="bg-gray-50 dark:bg-gray-800">
					<tr>
						<th className="px-4 py-3 text-left text-xs font-medium uppercase tracking-wider text-gray-500">
							Key
						</th>
						<th className="px-4 py-3 text-left text-xs font-medium uppercase tracking-wider text-gray-500">
							Value
						</th>
						<th className="px-4 py-3 text-right text-xs font-medium uppercase tracking-wider text-gray-500">
							Actions
						</th>
					</tr>
				</thead>
				<tbody className="divide-y divide-gray-200 bg-white dark:divide-gray-700 dark:bg-gray-900">
					{entries.length === 0 ? (
						<tr>
							<td
								colSpan={3}
								className="px-4 py-8 text-center text-sm text-gray-500"
							>
								No configuration entries.
							</td>
						</tr>
					) : (
						entries.map(([key, value]) => (
							<tr key={key}>
								<td className="whitespace-nowrap px-4 py-3 font-mono text-sm text-gray-900 dark:text-gray-100">
									{key}
								</td>
								<td className="px-4 py-3 text-sm text-gray-700 dark:text-gray-300">
									{editingKey === key ? (
										<input
											type="text"
											value={editValue}
											onChange={(e) => setEditValue(e.target.value)}
											className="w-full rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700 dark:text-gray-100"
											autoFocus
										/>
									) : (
										String(value)
									)}
								</td>
								<td className="whitespace-nowrap px-4 py-3 text-right text-sm">
									{editingKey === key ? (
										<div className="flex justify-end gap-2">
											<button
												onClick={() =>
													updateConfig.mutate({ key, value: editValue })
												}
												disabled={updateConfig.isPending}
												className="inline-flex items-center gap-1 text-blue-600 hover:text-blue-700"
											>
												<Save className="h-3 w-3" /> Save
											</button>
											<button
												onClick={() => setEditingKey(null)}
												className="text-gray-500 hover:text-gray-700"
											>
												Cancel
											</button>
										</div>
									) : (
										<button
											onClick={() => {
												setEditingKey(key);
												setEditValue(String(value));
											}}
											className="text-blue-600 hover:text-blue-700"
										>
											Edit
										</button>
									)}
								</td>
							</tr>
						))
					)}
				</tbody>
			</table>
		</div>
	);
}

function ProvidersTab({ tenantSlug }: { tenantSlug: string }) {
	const { data, isLoading, error, refetch } = useQuery<
		Array<{ id: string; provider_type: string; name: string; status: string }>
	>({
		queryKey: ["tenant", tenantSlug, "providers"],
		queryFn: () =>
			apiClient.get(`/api/v1/admin/tenants/${tenantSlug}/providers`),
	});

	if (isLoading) {
		return (
			<div className="flex justify-center p-8">
				<Loader2 className="h-6 w-6 animate-spin text-gray-400" />
			</div>
		);
	}

	if (error) {
		return (
			<div className="p-8 text-center text-red-600">
				Failed to load providers.{" "}
				<button onClick={() => refetch()} className="underline">
					Retry
				</button>
			</div>
		);
	}

	const providers = Array.isArray(data) ? data : [];

	return (
		<div className="space-y-4">
			{providers.length === 0 ? (
				<p className="py-8 text-center text-sm text-gray-500">
					No providers configured.
				</p>
			) : (
				providers.map((p) => (
					<div
						key={p.id}
						className="flex items-center justify-between rounded-lg border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-800"
					>
						<div>
							<div className="font-medium text-gray-900 dark:text-gray-100">
								{p.name}
							</div>
							<div className="text-sm text-gray-500 dark:text-gray-400">
								{p.provider_type}
							</div>
						</div>
						<div className="flex items-center gap-3">
							<span
								className={cn(
									"inline-flex rounded-full px-2 py-0.5 text-xs font-medium",
									p.status === "active"
										? "bg-green-100 text-green-700 dark:bg-green-900 dark:text-green-300"
										: "bg-gray-100 text-gray-700 dark:bg-gray-700 dark:text-gray-300",
								)}
							>
								{p.status}
							</span>
							<button className="text-sm text-blue-600 hover:text-blue-700">
								Configure
							</button>
						</div>
					</div>
				))
			)}
		</div>
	);
}

function RepositoryTab({ tenantSlug }: { tenantSlug: string }) {
	const { data, isLoading, error, refetch } = useQuery<{
		repository_url?: string;
		branch?: string;
		path?: string;
		status?: string;
	}>({
		queryKey: ["tenant", tenantSlug, "repository"],
		queryFn: () =>
			apiClient.get(`/api/v1/admin/tenants/${tenantSlug}/repository-binding`),
	});

	if (isLoading) {
		return (
			<div className="flex justify-center p-8">
				<Loader2 className="h-6 w-6 animate-spin text-gray-400" />
			</div>
		);
	}

	if (error) {
		return (
			<div className="p-8 text-center text-red-600">
				Failed to load repository binding.{" "}
				<button onClick={() => refetch()} className="underline">
					Retry
				</button>
			</div>
		);
	}

	if (!data?.repository_url) {
		return (
			<p className="py-8 text-center text-sm text-gray-500">
				No repository binding configured.
			</p>
		);
	}

	return (
		<div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
			<div>
				<dt className="text-sm font-medium text-gray-500 dark:text-gray-400">
					Repository URL
				</dt>
				<dd className="mt-1 font-mono text-sm text-gray-900 dark:text-gray-100">
					{data.repository_url}
				</dd>
			</div>
			{data.branch && (
				<div>
					<dt className="text-sm font-medium text-gray-500 dark:text-gray-400">
						Branch
					</dt>
					<dd className="mt-1 text-sm text-gray-900 dark:text-gray-100">
						{data.branch}
					</dd>
				</div>
			)}
			{data.path && (
				<div>
					<dt className="text-sm font-medium text-gray-500 dark:text-gray-400">
						Path
					</dt>
					<dd className="mt-1 font-mono text-sm text-gray-900 dark:text-gray-100">
						{data.path}
					</dd>
				</div>
			)}
			{data.status && (
				<div>
					<dt className="text-sm font-medium text-gray-500 dark:text-gray-400">
						Status
					</dt>
					<dd className="mt-1">
						<span
							className={cn(
								"inline-flex rounded-full px-2 py-0.5 text-xs font-medium",
								data.status === "active"
									? "bg-green-100 text-green-700"
									: "bg-gray-100 text-gray-700",
							)}
						>
							{data.status}
						</span>
					</dd>
				</div>
			)}
		</div>
	);
}

export function Component() {
	return <TenantDetailPage />;
}

export default function TenantDetailPage() {
	const { id } = useParams();
	const navigate = useNavigate();
	const [activeTab, setActiveTab] = useState<Tab>("overview");

	// Backend returns `{ success: true, tenant: TenantRecord }` (see
	// cli/src/server/tenant_api.rs::show_tenant). Typing the query as
	// `TenantRecord` directly caused `tenant.slug` to be undefined, which
	// propagated into the Config/Providers/Repository tabs and produced
	// requests to `/api/v1/admin/tenants/undefined/*` (404s in the console).
	const { data, isLoading, error, refetch } = useQuery<{
		tenant: TenantRecord;
	}>({
		queryKey: ["tenant", id],
		queryFn: () => apiClient.get(`/api/v1/admin/tenants/${id}`),
		enabled: !!id,
	});
	const tenant = data?.tenant;

	const tabs: { key: Tab; label: string }[] = [
		{ key: "overview", label: "Overview" },
		{ key: "config", label: "Config" },
		{ key: "providers", label: "Providers" },
		{ key: "repository", label: "Repository" },
		{ key: "manifest", label: "Manifest" },
	];

	return (
		<div>
			<div className="mb-6 flex items-center gap-3">
				<button
					onClick={() => navigate("/admin/tenants")}
					className="text-gray-400 hover:text-gray-600"
				>
					<ArrowLeft className="h-5 w-5" />
				</button>
				<Building2 className="h-6 w-6 text-gray-400" />
				<h1 className="text-2xl font-semibold text-gray-900 dark:text-gray-100">
					{tenant?.name ?? "Tenant Detail"}
				</h1>
				{id && (
					<span className="rounded-md bg-gray-100 px-2 py-0.5 text-sm text-gray-500 dark:bg-gray-700 dark:text-gray-400">
						{id}
					</span>
				)}
			</div>

			{isLoading && (
				<div className="flex justify-center p-8">
					<Loader2 className="h-6 w-6 animate-spin text-gray-400" />
				</div>
			)}

			{error && (
				<div className="p-8 text-center text-red-600">
					Failed to load tenant.{" "}
					<button onClick={() => refetch()} className="underline">
						Retry
					</button>
				</div>
			)}

			{tenant && (
				<>
					<div className="mb-6 border-b border-gray-200 dark:border-gray-700">
						<nav className="-mb-px flex gap-6">
							{tabs.map((tab) => (
								<button
									key={tab.key}
									onClick={() => setActiveTab(tab.key)}
									className={cn(
										"border-b-2 pb-3 text-sm font-medium",
										activeTab === tab.key
											? "border-blue-500 text-blue-600 dark:text-blue-400"
											: "border-transparent text-gray-500 hover:border-gray-300 hover:text-gray-700 dark:text-gray-400",
									)}
								>
									{tab.label}
								</button>
							))}
						</nav>
					</div>

					<div className="rounded-lg border border-gray-200 bg-white p-6 dark:border-gray-700 dark:bg-gray-800">
						{activeTab === "overview" && <OverviewTab tenant={tenant} />}
						{activeTab === "config" && <ConfigTab tenantSlug={tenant.slug} />}
						{activeTab === "providers" && (
							<ProvidersTab tenantSlug={tenant.slug} />
						)}
						{activeTab === "repository" && (
							<RepositoryTab tenantSlug={tenant.slug} />
						)}
						{activeTab === "manifest" && (
							<ManifestPanel tenantSlug={tenant.slug} />
						)}
					</div>
				</>
			)}
		</div>
	);
}
