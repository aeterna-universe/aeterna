import {
	getStoredTokens,
	storeTokens,
	clearTokens,
	refreshTokens,
} from "@/auth/token-manager";
import type { ApiClientConfig, AuthTokens } from "./types";

interface SelectTenantOption {
	id: string;
	slug: string;
	name: string;
}

export interface SelectTenantRequestState {
	message: string;
	hint?: string;
	availableTenants: SelectTenantOption[];
	pendingCount: number;
}

interface QueuedSelectTenantRequest<T = unknown> {
	method: string;
	path: string;
	body?: unknown;
	resolve: (value: T | PromiseLike<T>) => void;
	reject: (reason?: unknown) => void;
}

class ApiClient {
	private baseUrl: string;
	private getTokens: () => AuthTokens | null;
	private onTokenRefresh: (tokens: AuthTokens) => void;
	private onUnauthorized: () => void;
	private activeTenantId: string | null = null;
	private targetTenantId: string | null = null;
	private refreshPromise: Promise<AuthTokens> | null = null;
	private pendingSelectTenant: {
		payload: Omit<SelectTenantRequestState, "pendingCount">;
		requests: QueuedSelectTenantRequest[];
	} | null = null;
	private selectTenantListeners = new Set<
		(state: SelectTenantRequestState | null) => void
	>();

	constructor(config: ApiClientConfig) {
		this.baseUrl = config.baseUrl;
		this.getTokens = config.getTokens;
		this.onTokenRefresh = config.onTokenRefresh;
		this.onUnauthorized = config.onUnauthorized;
	}

	setActiveTenant(tenantId: string | null) {
		this.activeTenantId = tenantId;
	}

	getActiveTenant(): string | null {
		return this.activeTenantId;
	}

	setTargetTenant(tenantId: string | null) {
		this.targetTenantId = tenantId;
	}

	getTargetTenant(): string | null {
		return this.targetTenantId;
	}

	subscribeSelectTenant(
		listener: (state: SelectTenantRequestState | null) => void,
	): () => void {
		this.selectTenantListeners.add(listener);
		listener(this.getSelectTenantState());
		return () => {
			this.selectTenantListeners.delete(listener);
		};
	}

	async resolveSelectTenant(tenantId: string): Promise<void> {
		const pending = this.pendingSelectTenant;
		this.pendingSelectTenant = null;
		this.emitSelectTenant();
		if (!pending) return;

		for (const request of pending.requests) {
			this.executeRequest(request.method, request.path, request.body, tenantId)
				.then(request.resolve)
				.catch(request.reject);
		}
	}

	dismissSelectTenant(reason = new Error("Tenant selection dismissed")): void {
		const pending = this.pendingSelectTenant;
		this.pendingSelectTenant = null;
		this.emitSelectTenant();
		pending?.requests.forEach((request) => request.reject(reason));
	}

	private getSelectTenantState(): SelectTenantRequestState | null {
		if (!this.pendingSelectTenant) return null;
		return {
			...this.pendingSelectTenant.payload,
			pendingCount: this.pendingSelectTenant.requests.length,
		};
	}

	private emitSelectTenant() {
		const state = this.getSelectTenantState();
		this.selectTenantListeners.forEach((listener) => listener(state));
	}

	private buildHeaders(
		tokens: AuthTokens | null,
		overrideTenantId?: string | null,
	): Record<string, string> {
		const headers: Record<string, string> = {
			"Content-Type": "application/json",
		};
		if (tokens) {
			headers["Authorization"] = `Bearer ${tokens.access_token}`;
		}
		const effectiveTenantId = overrideTenantId ?? this.activeTenantId;
		if (effectiveTenantId) {
			headers["X-Tenant-ID"] = effectiveTenantId;
		}
		if (this.targetTenantId) {
			headers["X-Target-Tenant-ID"] = this.targetTenantId;
		}
		return headers;
	}

	private async tryRefresh(): Promise<AuthTokens> {
		if (this.refreshPromise) {
			return this.refreshPromise;
		}

		const tokens = this.getTokens();
		if (!tokens?.refresh_token) {
			throw new Error("No refresh token available");
		}

		this.refreshPromise = refreshTokens(tokens.refresh_token)
			.then((newTokens) => {
				this.onTokenRefresh(newTokens);
				return newTokens;
			})
			.finally(() => {
				this.refreshPromise = null;
			});

		return this.refreshPromise;
	}

	private async parseResponseBody<T>(response: Response): Promise<T> {
		if (response.status === 204) {
			return undefined as T;
		}
		return response.json() as Promise<T>;
	}

	private async executeFetch(
		method: string,
		path: string,
		body?: unknown,
		overrideTenantId?: string | null,
	): Promise<Response> {
		const tokens = this.getTokens();
		const url = `${this.baseUrl}${path}`;

		const response = await fetch(url, {
			method,
			headers: this.buildHeaders(tokens, overrideTenantId),
			body: body ? JSON.stringify(body) : undefined,
		});

		if (response.status !== 401) {
			return response;
		}

		try {
			const newTokens = await this.tryRefresh();
			const retryResponse = await fetch(url, {
				method,
				headers: this.buildHeaders(newTokens, overrideTenantId),
				body: body ? JSON.stringify(body) : undefined,
			});

			if (retryResponse.status === 401) {
				this.onUnauthorized();
				throw new Error("Unauthorized after token refresh");
			}

			return retryResponse;
		} catch {
			this.onUnauthorized();
			throw new Error("Unauthorized");
		}
	}

	private queueSelectTenant<T>(
		payload: Omit<SelectTenantRequestState, "pendingCount">,
		method: string,
		path: string,
		body?: unknown,
	): Promise<T> {
		return new Promise<T>((resolve, reject) => {
			if (!this.pendingSelectTenant) {
				this.pendingSelectTenant = { payload, requests: [] };
			}
			this.pendingSelectTenant.requests.push({
				method,
				path,
				body,
				resolve: resolve as (value: unknown) => void,
				reject,
			});
			this.emitSelectTenant();
		});
	}

	async request<T>(method: string, path: string, body?: unknown): Promise<T> {
		return this.executeRequest<T>(method, path, body);
	}

	private async executeRequest<T>(
		method: string,
		path: string,
		body?: unknown,
		overrideTenantId?: string | null,
	): Promise<T> {
		const response = await this.executeFetch(
			method,
			path,
			body,
			overrideTenantId,
		);

		if (!response.ok) {
			let errorBody: unknown = null;
			try {
				errorBody = await response.json();
			} catch {
				// ignore JSON parse failures for non-JSON responses
			}

			if (
				response.status === 400 &&
				errorBody &&
				typeof errorBody === "object" &&
				"error" in errorBody &&
				(errorBody as { error?: string }).error === "select_tenant"
			) {
				const payload = errorBody as {
					message?: string;
					hint?: string;
					availableTenants?: SelectTenantOption[];
				};
				return this.queueSelectTenant<T>(
					{
						message: payload.message ?? "Select a tenant to continue",
						hint: payload.hint,
						availableTenants: payload.availableTenants ?? [],
					},
					method,
					path,
					body,
				);
			}

			if (
				response.status === 401 &&
				errorBody &&
				typeof errorBody === "object" &&
				"error" in errorBody &&
				(errorBody as { error?: string }).error === "wrong_audience"
			) {
				this.onUnauthorized();
				throw new Error("Session audience mismatch");
			}

			const message =
				errorBody &&
				typeof errorBody === "object" &&
				"message" in errorBody &&
				typeof (errorBody as { message?: string }).message === "string"
					? (errorBody as { message: string }).message
					: `API error: ${response.status} ${response.statusText}`;
			throw new Error(message);
		}

		return this.parseResponseBody<T>(response);
	}

	get<T>(path: string): Promise<T> {
		return this.request<T>("GET", path);
	}

	post<T>(path: string, body?: unknown): Promise<T> {
		return this.request<T>("POST", path, body);
	}

	put<T>(path: string, body?: unknown): Promise<T> {
		return this.request<T>("PUT", path, body);
	}

	patch<T>(path: string, body?: unknown): Promise<T> {
		return this.request<T>("PATCH", path, body);
	}

	delete<T>(path: string): Promise<T> {
		return this.request<T>("DELETE", path);
	}
}

export const apiClient = new ApiClient({
	baseUrl: "",
	getTokens: () => getStoredTokens(),
	onTokenRefresh: (tokens) => storeTokens(tokens),
	onUnauthorized: () => {
		clearTokens();
		window.location.href = "/admin/login";
	},
});
