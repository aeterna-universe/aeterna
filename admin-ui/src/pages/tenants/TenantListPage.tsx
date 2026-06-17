import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { Building2, Plus, Search, Loader2 } from "lucide-react";
import { cn } from "@/lib/utils";
import { apiClient } from "@/api/client";
import type { TenantRecord } from "@/api/types";
import { CreateTenantWizard } from "./wizard/CreateTenantWizard";

function TenantListPageContent() {
	const navigate = useNavigate();
	const [search, setSearch] = useState("");
	const [dialogOpen, setDialogOpen] = useState(false);

	const { data, isLoading, error, refetch } = useQuery<{
		tenants: TenantRecord[];
	}>({
		queryKey: ["tenants"],
		queryFn: () => apiClient.get("/api/v1/admin/tenants"),
	});

	const tenants = data?.tenants ?? [];
	const filtered = tenants.filter(
		(t) =>
			(t.name?.toLowerCase() ?? "").includes(search.toLowerCase()) ||
			(t.slug?.toLowerCase() ?? "").includes(search.toLowerCase()) ||
			(t.account?.name?.toLowerCase() ?? "").includes(search.toLowerCase()) ||
			(t.account?.slug?.toLowerCase() ?? "").includes(search.toLowerCase()) ||
			(t.environment?.toLowerCase() ?? "").includes(search.toLowerCase()),
	);

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
				Failed to load tenants.{" "}
				<button
					onClick={() => refetch()}
					className="underline hover:no-underline"
				>
					Retry
				</button>
			</div>
		);
	}

	return (
		<>
			<div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
				<div className="relative flex-1 sm:max-w-xs">
					<Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-gray-400" />
					<input
						type="text"
						value={search}
						onChange={(e) => setSearch(e.target.value)}
						placeholder="Search tenants..."
						className="w-full rounded-md border border-gray-300 py-2 pl-9 pr-3 text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-700 dark:text-gray-100"
					/>
				</div>
				<button
					onClick={() => setDialogOpen(true)}
					className="inline-flex items-center gap-2 rounded-md bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700"
				>
					<Plus className="h-4 w-4" />
					Create Tenant
				</button>
			</div>

			<div className="overflow-hidden rounded-lg border border-gray-200 dark:border-gray-700">
				<table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
					<thead className="bg-gray-50 dark:bg-gray-800">
						<tr>
							<th className="px-4 py-3 text-left text-xs font-medium uppercase tracking-wider text-gray-500 dark:text-gray-400">
								Name
							</th>
							<th className="px-4 py-3 text-left text-xs font-medium uppercase tracking-wider text-gray-500 dark:text-gray-400">
								Slug
							</th>
							<th className="px-4 py-3 text-left text-xs font-medium uppercase tracking-wider text-gray-500 dark:text-gray-400">
								Account
							</th>
							<th className="px-4 py-3 text-left text-xs font-medium uppercase tracking-wider text-gray-500 dark:text-gray-400">
								Environment
							</th>
							<th className="px-4 py-3 text-left text-xs font-medium uppercase tracking-wider text-gray-500 dark:text-gray-400">
								Status
							</th>
							<th className="px-4 py-3 text-left text-xs font-medium uppercase tracking-wider text-gray-500 dark:text-gray-400">
								Created
							</th>
						</tr>
					</thead>
					<tbody className="divide-y divide-gray-200 bg-white dark:divide-gray-700 dark:bg-gray-900">
						{filtered.length === 0 ? (
							<tr>
								<td
									colSpan={6}
									className="px-4 py-8 text-center text-sm text-gray-500"
								>
									No tenants found.
								</td>
							</tr>
						) : (
							filtered.map((tenant) => (
								<tr
									key={tenant.id}
									onClick={() => navigate(`/admin/tenants/${tenant.slug}`)}
									className="cursor-pointer hover:bg-gray-50 dark:hover:bg-gray-800"
								>
									<td className="whitespace-nowrap px-4 py-3 text-sm font-medium text-gray-900 dark:text-gray-100">
										{tenant.name}
									</td>
									<td className="whitespace-nowrap px-4 py-3 text-sm text-gray-500 dark:text-gray-400">
										{tenant.slug}
									</td>
									<td className="whitespace-nowrap px-4 py-3 text-sm text-gray-500 dark:text-gray-400">
										{tenant.account ? (
											<div>
												<div className="font-medium text-gray-900 dark:text-gray-100">
													{tenant.account.name}
												</div>
												<div className="text-xs">{tenant.account.slug}</div>
											</div>
										) : (
											<span>—</span>
										)}
									</td>
									<td className="whitespace-nowrap px-4 py-3 text-sm text-gray-500 dark:text-gray-400">
										{tenant.environment ?? "—"}
									</td>
									<td className="whitespace-nowrap px-4 py-3 text-sm">
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
									</td>
									<td className="whitespace-nowrap px-4 py-3 text-sm text-gray-500 dark:text-gray-400">
										{new Date(tenant.createdAt).toLocaleDateString()}
									</td>
								</tr>
							))
						)}
					</tbody>
				</table>
			</div>

			<CreateTenantWizard
				open={dialogOpen}
				onClose={() => setDialogOpen(false)}
			/>
		</>
	);
}

export function Component() {
	return <TenantListPage />;
}

export default function TenantListPage() {
	return (
		<div>
			<div className="mb-6 flex items-center gap-3">
				<Building2 className="h-6 w-6 text-gray-400" />
				<h1 className="text-2xl font-semibold text-gray-900 dark:text-gray-100">
					Tenants
				</h1>
			</div>
			<TenantListPageContent />
		</div>
	);
}
