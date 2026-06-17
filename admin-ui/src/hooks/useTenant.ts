import { useCallback, useMemo } from "react";
import { useAuth } from "@/auth/AuthContext";
import type { TenantRecord } from "@/api/types";

const TENANT_SOURCE_LABELS = {
	"local-session": "from this browser session",
	"server-default": "from server default",
	"single-membership": "single membership",
	"admin-impersonation": "impersonating",
	"explicit-selection": "selected here",
} as const;

export function useTenant() {
	const {
		tenants,
		activeTenantId,
		activeTenantSource,
		setActiveTenant,
		isPlatformAdmin,
	} = useAuth();

	const activeTenant: TenantRecord | null = useMemo(
		() => tenants.find((t) => t.id === activeTenantId) ?? null,
		[tenants, activeTenantId],
	);

	const switchTenant = useCallback(
		(tenantId: string) => {
			const tenant = tenants.find((t) => t.id === tenantId);
			if (tenant) {
				setActiveTenant(tenantId, "explicit-selection");
			}
		},
		[tenants, setActiveTenant],
	);

	const clearTenant = useCallback(() => {
		setActiveTenant(null);
	}, [setActiveTenant]);

	return {
		tenants,
		activeTenant,
		activeTenantId,
		tenantSource: activeTenantSource,
		tenantSourceLabel: activeTenantSource
			? TENANT_SOURCE_LABELS[activeTenantSource]
			: null,
		isPlatformAdmin,
		switchTenant,
		clearTenant,
	};
}
