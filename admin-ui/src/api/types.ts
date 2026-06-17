export interface AccountRef {
	id: string;
	slug: string;
	name: string;
}

export interface TenantRecord {
	id: string;
	slug: string;
	name: string;
	status: string;
	account?: AccountRef | null;
	environment?: string | null;
	createdAt: string;
	updatedAt: string;
}

export interface OrganizationalUnit {
	id: string;
	name: string;
	unitType: string;
	parentId: string | null;
	tenantId: string;
	metadata: Record<string, unknown>;
}

export interface UserRecord {
	id: string;
	email: string;
	name: string;
	avatarUrl: string | null;
	status: string;
}

export type Role =
	| "PlatformAdmin"
	| "TenantAdmin"
	| "Admin"
	| "Architect"
	| "TechLead"
	| "Developer"
	| "Viewer"
	| "Agent";

export interface RoleAssignment {
	user_id: string;
	role: Role;
	tenant_id: string;
	resource_type: string | null;
	resource_id: string | null;
}

export type MemoryLayer =
	| "Agent"
	| "User"
	| "Session"
	| "Project"
	| "Team"
	| "Org"
	| "Tenant";

export interface MemoryEntry {
	id: string;
	content: string;
	layer: MemoryLayer;
	importanceScore: number;
	metadata: Record<string, unknown>;
	createdAt: string;
	updatedAt: string;
}

export type KnowledgeLayer = "Tenant" | "Organization" | "Team" | "Project";

export type KnowledgeType = "Adr" | "Policy" | "Pattern" | "Spec" | "Hindsight";

export type KnowledgeStatus =
	| "Draft"
	| "Proposed"
	| "Accepted"
	| "Deprecated"
	| "Superseded"
	| "Rejected";

export interface KnowledgeEntry {
	id: string;
	path: string;
	content: string;
	layer: KnowledgeLayer;
	kind: KnowledgeType;
	status: KnowledgeStatus;
	metadata: Record<string, unknown>;
	commit_hash: string | null;
	author: string | null;
	updated_at: string;
}

export interface GovernanceRequest {
	id: string;
	request_type: string;
	status: string;
	requestor_id: string;
	created_at: string;
}

export interface GovernanceEvent {
	id: string;
	action: string;
	actor: string;
	resource_type: string;
	resource_id: string | null;
	timestamp: string;
	details: Record<string, unknown>;
}

export interface PolicyRecord {
	id: string;
	name: string;
	description: string;
	layer: string;
	mode: string;
	rules: Record<string, unknown>;
}

export interface ComponentHealth {
	status: string;
	message?: string;
}

export interface HealthResponse {
	status: string;
	components: Record<string, ComponentHealth>;
}

export interface ReadinessResponse {
	ready: boolean;
	checks: Record<string, boolean>;
}

export interface UserProfile {
	user_id: string;
	github_login: string;
	email: string;
	avatar_url: string | null;
}

export interface AuthTokens {
	access_token: string;
	refresh_token: string;
	expires_in: number;
	/** Epoch seconds when the access token was stored */
	stored_at?: number;
}

export type TenantSource =
	| "local-session"
	| "server-default"
	| "single-membership"
	| "admin-impersonation"
	| "explicit-selection";

export interface AdminSession {
	user: UserProfile;
	roles: RoleAssignment[];
	tenants: TenantRecord[];
	is_platform_admin: boolean;
	is_tenant_admin?: boolean;
	active_tenant_id?: string | null;
	default_tenant_id?: string | null;
	default_tenant_slug?: string | null;
}

export interface ApiClientConfig {
	baseUrl: string;
	getTokens: () => AuthTokens | null;
	onTokenRefresh: (tokens: AuthTokens) => void;
	onUnauthorized: () => void;
}
