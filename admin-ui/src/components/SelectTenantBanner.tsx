import { useEffect, useMemo, useState } from "react";
import { Building2, Info, X } from "lucide-react";
import { apiClient, type SelectTenantRequestState } from "@/api/client";
import { useTenant } from "@/hooks/useTenant";

const REMEMBER_KEY = "ui.tenantSelector.remember";

function loadRememberPreference(): boolean {
	try {
		const raw = localStorage.getItem(REMEMBER_KEY);
		return raw === null ? true : raw === "true";
	} catch {
		return true;
	}
}

export function SelectTenantBanner() {
	const { switchTenant } = useTenant();
	const [pending, setPending] = useState<SelectTenantRequestState | null>(null);
	const [tenantId, setTenantId] = useState<string | null>(null);
	const [rememberAcrossDevices, setRememberAcrossDevices] = useState(
		loadRememberPreference,
	);
	const [error, setError] = useState<string | null>(null);

	useEffect(() => apiClient.subscribeSelectTenant(setPending), []);

	useEffect(() => {
		try {
			localStorage.setItem(REMEMBER_KEY, String(rememberAcrossDevices));
		} catch {
			// ignore storage failures
		}
	}, [rememberAcrossDevices]);

	const effectiveTenantId = useMemo(() => {
		if (!pending) return "";
		if (
			tenantId &&
			pending.availableTenants.some((tenant) => tenant.id === tenantId)
		) {
			return tenantId;
		}
		return pending.availableTenants[0]?.id ?? "";
	}, [pending, tenantId]);

	const selectedTenant = useMemo(
		() =>
			pending?.availableTenants.find(
				(tenant) => tenant.id === effectiveTenantId,
			) ?? null,
		[effectiveTenantId, pending],
	);

	if (!pending) return null;

	const onSelect = async () => {
		if (!selectedTenant) return;

		try {
			switchTenant(selectedTenant.id);
			if (rememberAcrossDevices) {
				await apiClient.put("/api/v1/user/me/default-tenant", {
					tenantId: selectedTenant.slug,
				});
			}
			await apiClient.resolveSelectTenant(selectedTenant.id);
		} catch (err) {
			setError(err instanceof Error ? err.message : "Failed to select tenant");
		}
	};

	return (
		<div className="border-b border-amber-200 bg-amber-50 px-4 py-3 text-sm text-amber-950">
			<div className="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
				<div className="flex min-w-0 items-start gap-3">
					<Info className="mt-0.5 h-4 w-4 flex-shrink-0" />
					<div className="min-w-0">
						<div className="font-medium">
							{pending.message ?? "Select a tenant to continue"}
						</div>
						{pending.hint && (
							<div className="text-amber-900/80">{pending.hint}</div>
						)}
						{error && <div className="mt-1 text-red-700">{error}</div>}
					</div>
				</div>

				<div className="flex flex-col gap-2 md:min-w-[360px]">
					<div className="flex gap-2">
						<div className="flex flex-1 items-center gap-2 rounded-md border border-amber-300 bg-white px-3 py-2">
							<Building2 className="h-4 w-4 text-amber-700" />
							<select
								value={effectiveTenantId}
								onChange={(event) => {
									setTenantId(event.target.value);
									setError(null);
								}}
								className="w-full bg-transparent outline-none"
							>
								{pending.availableTenants.map((tenant) => (
									<option key={tenant.id} value={tenant.id}>
										{tenant.name} ({tenant.slug})
									</option>
								))}
							</select>
						</div>
						<button
							type="button"
							onClick={onSelect}
							className="rounded-md bg-amber-900 px-3 py-2 font-medium text-white hover:bg-amber-800"
						>
							Select
						</button>
						<button
							type="button"
							onClick={() =>
								apiClient.dismissSelectTenant(
									new Error("Tenant selection dismissed"),
								)
							}
							className="rounded-md border border-amber-300 px-3 py-2 hover:bg-amber-100"
							aria-label="Dismiss tenant selector"
						>
							<X className="h-4 w-4" />
						</button>
					</div>

					<label className="flex items-center gap-2 text-xs text-amber-900/80">
						<input
							type="checkbox"
							checked={rememberAcrossDevices}
							onChange={(event) =>
								setRememberAcrossDevices(event.target.checked)
							}
						/>
						Remember across devices
					</label>
				</div>
			</div>
		</div>
	);
}
