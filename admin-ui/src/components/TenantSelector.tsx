import { useState, useRef, useEffect } from "react";
import { Building2, ChevronDown, Search, XCircle } from "lucide-react";
import { cn } from "@/lib/utils";
import { useTenant } from "@/hooks/useTenant";
import { apiClient } from "@/api/client";

const REMEMBER_KEY = "ui.tenantSelector.remember";

function loadRememberPreference(): boolean {
	try {
		const raw = localStorage.getItem(REMEMBER_KEY);
		return raw === null ? true : raw === "true";
	} catch {
		return true;
	}
}

export function TenantSelector() {
	const { tenants, activeTenant, isPlatformAdmin, switchTenant, clearTenant } =
		useTenant();
	const [isOpen, setIsOpen] = useState(false);
	const [search, setSearch] = useState("");
	const [rememberAcrossDevices, setRememberAcrossDevices] = useState(
		loadRememberPreference,
	);
	const [error, setError] = useState<string | null>(null);
	const dropdownRef = useRef<HTMLDivElement>(null);

	useEffect(() => {
		function handleClickOutside(event: MouseEvent) {
			if (
				dropdownRef.current &&
				!dropdownRef.current.contains(event.target as Node)
			) {
				setIsOpen(false);
				setSearch("");
				setError(null);
			}
		}
		document.addEventListener("mousedown", handleClickOutside);
		return () => document.removeEventListener("mousedown", handleClickOutside);
	}, []);

	useEffect(() => {
		try {
			localStorage.setItem(REMEMBER_KEY, String(rememberAcrossDevices));
		} catch {
			// ignore storage failures
		}
	}, [rememberAcrossDevices]);

	const filteredTenants = tenants.filter(
		(t) =>
			t.name.toLowerCase().includes(search.toLowerCase()) ||
			t.slug.toLowerCase().includes(search.toLowerCase()) ||
			(t.account?.name ?? "").toLowerCase().includes(search.toLowerCase()) ||
			(t.account?.slug ?? "").toLowerCase().includes(search.toLowerCase()) ||
			(t.environment ?? "").toLowerCase().includes(search.toLowerCase()),
	);

	const changeTenant = async (tenantId: string) => {
		const previousTenantId = activeTenant?.id ?? null;
		setError(null);
		switchTenant(tenantId);

		try {
			const tenant = tenants.find((entry) => entry.id === tenantId);
			if (rememberAcrossDevices && tenant) {
				await apiClient.put("/api/v1/user/me/default-tenant", {
					tenantId: tenant.slug,
				});
			}
			setIsOpen(false);
			setSearch("");
		} catch (err) {
			if (previousTenantId) {
				switchTenant(previousTenantId);
			} else {
				clearTenant();
			}
			setError(
				err instanceof Error
					? err.message
					: "Failed to persist tenant selection",
			);
		}
	};

	const clearActiveTenant = async () => {
		setError(null);
		const previousTenantId = activeTenant?.id ?? null;
		clearTenant();

		try {
			if (rememberAcrossDevices) {
				await apiClient.delete("/api/v1/user/me/default-tenant");
			}
			setIsOpen(false);
			setSearch("");
		} catch (err) {
			if (previousTenantId) {
				switchTenant(previousTenantId);
			}
			setError(
				err instanceof Error ? err.message : "Failed to clear tenant selection",
			);
		}
	};

	return (
		<div className="relative" ref={dropdownRef}>
			<button
				onClick={() => setIsOpen(!isOpen)}
				className="flex items-center gap-2 rounded-md border border-gray-200 bg-white px-3 py-1.5 text-sm text-gray-700 transition-colors hover:bg-gray-50"
			>
				<Building2 className="h-4 w-4 text-gray-400" />
				<span className="max-w-[200px] truncate">
					{activeTenant
						? `${activeTenant.name}${activeTenant.environment ? ` (${activeTenant.environment})` : ""}`
						: "Select tenant"}
				</span>
				<ChevronDown className="h-3.5 w-3.5 text-gray-400" />
			</button>

			{isOpen && (
				<div className="absolute right-0 top-full z-50 mt-1 w-72 rounded-md border border-gray-200 bg-white shadow-lg">
					{isPlatformAdmin && (
						<div className="border-b border-gray-100 p-2">
							<div className="flex items-center gap-2 rounded-md border border-gray-200 px-2 py-1.5">
								<Search className="h-3.5 w-3.5 text-gray-400" />
								<input
									type="text"
									placeholder="Search tenants..."
									value={search}
									onChange={(e) => setSearch(e.target.value)}
									className="w-full bg-transparent text-sm outline-none placeholder:text-gray-400"
									autoFocus
								/>
							</div>
						</div>
					)}

					<div className="max-h-60 overflow-y-auto p-1">
						{filteredTenants.length === 0 ? (
							<div className="px-3 py-2 text-sm text-gray-400">
								No tenants found
							</div>
						) : (
							filteredTenants.map((tenant) => (
								<button
									key={tenant.id}
									onClick={() => void changeTenant(tenant.id)}
									className={cn(
										"flex w-full items-center gap-2 rounded-md px-3 py-2 text-left text-sm transition-colors",
										tenant.id === activeTenant?.id
											? "bg-gray-100 text-gray-900"
											: "text-gray-600 hover:bg-gray-50",
									)}
								>
									<Building2 className="h-4 w-4 flex-shrink-0 text-gray-400" />
									<div className="min-w-0 flex-1">
										<div className="truncate font-medium">{tenant.name}</div>
										<div className="truncate text-xs text-gray-400">
											{tenant.slug}
											{tenant.environment ? ` • ${tenant.environment}` : ""}
											{tenant.account ? ` • ${tenant.account.slug}` : ""}
										</div>
									</div>
								</button>
							))
						)}
					</div>

					<div className="border-t border-gray-100 p-3 text-xs text-gray-500">
						<label className="flex items-center gap-2">
							<input
								type="checkbox"
								checked={rememberAcrossDevices}
								onChange={(event) =>
									setRememberAcrossDevices(event.target.checked)
								}
							/>
							Remember across devices
						</label>

						{isPlatformAdmin && activeTenant && (
							<button
								type="button"
								onClick={() => void clearActiveTenant()}
								className="mt-3 flex items-center gap-2 text-red-600 hover:text-red-700"
							>
								<XCircle className="h-4 w-4" />
								Clear active tenant
							</button>
						)}

						{error && <div className="mt-2 text-red-600">{error}</div>}
					</div>
				</div>
			)}
		</div>
	);
}
