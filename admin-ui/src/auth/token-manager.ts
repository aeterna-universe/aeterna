import type { AuthTokens } from "@/api/types";

const TOKEN_KEY = "aeterna_tokens";

export function getStoredTokens(): AuthTokens | null {
	try {
		const raw = localStorage.getItem(TOKEN_KEY);
		if (!raw) return null;
		return JSON.parse(raw) as AuthTokens;
	} catch {
		return null;
	}
}

export function storeTokens(tokens: AuthTokens): void {
	const withTimestamp: AuthTokens = {
		...tokens,
		stored_at: Math.floor(Date.now() / 1000),
	};
	localStorage.setItem(TOKEN_KEY, JSON.stringify(withTimestamp));
}

export function clearTokens(): void {
	localStorage.removeItem(TOKEN_KEY);
}

export function isTokenExpired(tokens: AuthTokens): boolean {
	if (!tokens.stored_at) return true;
	const now = Math.floor(Date.now() / 1000);
	return now >= tokens.stored_at + tokens.expires_in;
}

export function shouldRefresh(tokens: AuthTokens): boolean {
	if (!tokens.stored_at) return true;
	const now = Math.floor(Date.now() / 1000);
	return now >= tokens.stored_at + tokens.expires_in - 60;
}

export async function refreshTokens(refreshToken: string): Promise<AuthTokens> {
	const res = await fetch("/api/v1/auth/admin-ui/refresh", {
		method: "POST",
		headers: {
			Authorization: `Bearer ${refreshToken}`,
			"Content-Type": "application/json",
		},
	});
	if (!res.ok) throw new Error("Token refresh failed");
	return res.json();
}
