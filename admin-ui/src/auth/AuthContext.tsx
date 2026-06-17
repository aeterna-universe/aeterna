import {
	createContext,
	useContext,
	useEffect,
	useState,
	useCallback,
	useRef,
	type ReactNode,
} from "react";
import type {
	AuthTokens,
	UserProfile,
	TenantRecord,
	RoleAssignment,
	AdminSession,
	TenantSource,
} from "@/api/types";
import {
	getStoredTokens,
	storeTokens,
	clearTokens,
	isTokenExpired,
	shouldRefresh,
	refreshTokens,
} from "@/auth/token-manager";
import { apiClient } from "@/api/client";

const ACTIVE_TENANT_STORAGE_KEY = "ui.activeTenantId";

interface AuthState {
	user: UserProfile | null;
	tokens: AuthTokens | null;
	tenants: TenantRecord[];
	roles: RoleAssignment[];
	isAuthenticated: boolean;
	isPlatformAdmin: boolean;
	isTenantAdmin: boolean;
	activeTenantId: string | null;
	activeTenantSource: TenantSource | null;
	defaultTenantId: string | null;
	defaultTenantSlug: string | null;
	isLoading: boolean;
}

interface AuthContextValue extends AuthState {
	login: (tokens: AuthTokens) => Promise<void>;
	logout: () => void;
	setActiveTenant: (tenantId: string | null, source?: TenantSource) => void;
}

const AuthContext = createContext<AuthContextValue | null>(null);

const initialState: AuthState = {
	user: null,
	tokens: null,
	tenants: [],
	roles: [],
	isAuthenticated: false,
	isPlatformAdmin: false,
	isTenantAdmin: false,
	activeTenantId: null,
	activeTenantSource: null,
	defaultTenantId: null,
	defaultTenantSlug: null,
	isLoading: true,
};

function getStoredActiveTenantId(): string | null {
	try {
		return sessionStorage.getItem(ACTIVE_TENANT_STORAGE_KEY);
	} catch {
		return null;
	}
}

function persistActiveTenantId(tenantId: string | null) {
	try {
		if (tenantId) {
			sessionStorage.setItem(ACTIVE_TENANT_STORAGE_KEY, tenantId);
		} else {
			sessionStorage.removeItem(ACTIVE_TENANT_STORAGE_KEY);
		}
	} catch {
		// ignore storage failures
	}
}

function resolveInitialTenant(session: AdminSession): {
	tenantId: string | null;
	source: TenantSource | null;
} {
	const knownTenantIds = new Set(session.tenants.map((tenant) => tenant.id));
	const storedTenantId = getStoredActiveTenantId();
	if (storedTenantId && knownTenantIds.has(storedTenantId)) {
		return { tenantId: storedTenantId, source: "local-session" };
	}

	if (
		session.default_tenant_id &&
		knownTenantIds.has(session.default_tenant_id)
	) {
		return { tenantId: session.default_tenant_id, source: "server-default" };
	}

	if (session.tenants.length === 1) {
		return { tenantId: session.tenants[0].id, source: "single-membership" };
	}

	if (
		session.active_tenant_id &&
		knownTenantIds.has(session.active_tenant_id)
	) {
		return {
			tenantId: session.active_tenant_id,
			source: "admin-impersonation",
		};
	}

	return { tenantId: null, source: null };
}

export function AuthProvider({ children }: { children: ReactNode }) {
	const [state, setState] = useState<AuthState>(initialState);
	const refreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

	const clearRefreshTimer = useCallback(() => {
		if (refreshTimerRef.current) {
			clearTimeout(refreshTimerRef.current);
			refreshTimerRef.current = null;
		}
	}, []);

	const scheduleRefresh = useCallback(
		function schedule(tokens: AuthTokens) {
			clearRefreshTimer();
			if (!tokens.stored_at) return;

			const now = Math.floor(Date.now() / 1000);
			const expiresAt = tokens.stored_at + tokens.expires_in;
			const refreshAt = Math.max(expiresAt - 60, now + 10);
			const delayMs = (refreshAt - now) * 1000;

			refreshTimerRef.current = setTimeout(async () => {
				try {
					const newTokens = await refreshTokens(tokens.refresh_token);
					storeTokens(newTokens);
					setState((prev) => ({ ...prev, tokens: newTokens }));
					schedule(newTokens);
				} catch {
					clearTokens();
					setState({ ...initialState, isLoading: false });
				}
			}, delayMs);
		},
		[clearRefreshTimer],
	);

	const fetchSession = useCallback(async (): Promise<AdminSession> => {
		return apiClient.get<AdminSession>("/api/v1/auth/session");
	}, []);

	const setActiveTenant = useCallback(
		(tenantId: string | null, source: TenantSource = "explicit-selection") => {
			apiClient.setActiveTenant(tenantId);
			persistActiveTenantId(tenantId);
			setState((prev) => ({
				...prev,
				activeTenantId: tenantId,
				activeTenantSource: tenantId ? source : null,
			}));
		},
		[],
	);

	const login = useCallback(
		async (tokens: AuthTokens) => {
			storeTokens(tokens);
			try {
				const session = await fetchSession();
				const isPlatformAdmin = session.is_platform_admin;
				const isTenantAdmin =
					isPlatformAdmin ||
					session.roles.some((r) => r.role === "TenantAdmin");
				const initialTenant = resolveInitialTenant(session);

				apiClient.setActiveTenant(initialTenant.tenantId);

				setState({
					user: session.user,
					tokens,
					tenants: session.tenants,
					roles: session.roles,
					isAuthenticated: true,
					isPlatformAdmin,
					isTenantAdmin,
					activeTenantId: initialTenant.tenantId,
					activeTenantSource: initialTenant.source,
					defaultTenantId: session.default_tenant_id ?? null,
					defaultTenantSlug: session.default_tenant_slug ?? null,
					isLoading: false,
				});
				persistActiveTenantId(initialTenant.tenantId);
				scheduleRefresh(tokens);
			} catch {
				clearTokens();
				setState({ ...initialState, isLoading: false });
			}
		},
		[fetchSession, scheduleRefresh],
	);

	const logout = useCallback(() => {
		const refreshToken = state.tokens?.refresh_token;
		if (refreshToken) {
			void apiClient.post("/api/v1/auth/admin-ui/revoke", {
				refresh_token: refreshToken,
			});
		}
		clearRefreshTimer();
		clearTokens();
		persistActiveTenantId(null);
		apiClient.setActiveTenant(null);
		apiClient.setTargetTenant(null);
		setState({ ...initialState, isLoading: false });
	}, [clearRefreshTimer, state.tokens]);

	useEffect(() => {
		const tokens = getStoredTokens();
		if (!tokens || isTokenExpired(tokens)) {
			clearTokens();
			setState({ ...initialState, isLoading: false });
			return;
		}

		if (shouldRefresh(tokens)) {
			refreshTokens(tokens.refresh_token)
				.then((newTokens) => {
					storeTokens(newTokens);
					return login(newTokens);
				})
				.catch(() => {
					clearTokens();
					setState({ ...initialState, isLoading: false });
				});
		} else {
			login(tokens).catch(() => {
				setState({ ...initialState, isLoading: false });
			});
		}

		return () => clearRefreshTimer();
	}, [clearRefreshTimer, login]);

	return (
		<AuthContext.Provider value={{ ...state, login, logout, setActiveTenant }}>
			{children}
		</AuthContext.Provider>
	);
}

// eslint-disable-next-line react-refresh/only-export-components
export function useAuth(): AuthContextValue {
	const ctx = useContext(AuthContext);
	if (!ctx) {
		throw new Error("useAuth must be used within an AuthProvider");
	}
	return ctx;
}
