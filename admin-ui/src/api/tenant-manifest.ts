/**
 * Client-side TypeScript shape for the `TenantManifest` document accepted
 * by `POST /api/v1/admin/tenants/provision`.
 *
 * Wire contract source of truth: `cli/src/server/tenant_api.rs`
 * (manifest_v1 typed structs). This file intentionally mirrors the
 * *structural* subset the Admin UI wizard composes.
 *
 * §12.1 — harden-tenant-provisioning.
 */

export type SecretReferenceKind =
	| "inline"
	| "env"
	| "file"
	| "k8s"
	| "postgres";

export interface ManifestSecretReference {
	logicalName: string;
	kind: SecretReferenceKind;
	/** `inline`: plaintext — only accepted with server+caller opt-in. */
	inline?: string;
	/** `env`: environment variable name. */
	var?: string;
	/** `file`: absolute path on the server filesystem. */
	path?: string;
	/** `k8s`: Kubernetes Secret name. */
	name?: string;
	/** `k8s`: key within the Kubernetes Secret. */
	key?: string;
	/** `k8s`: optional namespace (empty = server default). */
	namespace?: string;
	/** `postgres`: opaque secret id + logical label. */
	secretId?: string;
	label?: string;
}

export interface ManifestMemberRef {
	userId?: string;
	email?: string;
	role?: string;
}

export interface ManifestUnitNode {
	name: string;
	unitType: "Organization" | "Team" | "Project";
	children?: ManifestUnitNode[];
	members?: ManifestMemberRef[];
}

export interface ManifestRoleAssignment {
	userId?: string;
	email?: string;
	role: string;
	/** Unit path (empty = tenant-wide). */
	unitPath?: string[];
}

export interface ManifestProvider {
	kind: string;
	model?: string;
	config?: Record<string, string>;
}

export interface ManifestProviders {
	llm?: ManifestProvider;
	embedding?: ManifestProvider;
	memoryLayers?: Record<string, unknown>;
}

export interface TenantManifest {
	apiVersion: "aeterna.io/v1";
	kind: "TenantManifest";
	metadata: {
		labels?: Record<string, string>;
		annotations?: Record<string, string>;
		generation?: number;
	};
	tenant: {
		slug: string;
		name: string;
		accountRef?: string;
		environment?: string;
		domainMappings?: string[];
	};
	config?: {
		fields?: Record<string, string>;
		secretReferences?: Record<string, ManifestSecretReference>;
	};
	secrets?: Array<{
		logicalName: string;
		secretValue: string;
	}>;
	hierarchy?: ManifestUnitNode[];
	roles?: ManifestRoleAssignment[];
	providers?: ManifestProviders;
}

export function emptyManifest(): TenantManifest {
	return {
		apiVersion: "aeterna.io/v1",
		kind: "TenantManifest",
		metadata: {},
		tenant: { slug: "", name: "" },
		config: { fields: {}, secretReferences: {} },
		secrets: [],
		hierarchy: [],
		roles: [],
		providers: {},
	};
}

/** Strip empty sub-sections so preview YAML/JSON is readable. */
export function pruneManifest(m: TenantManifest): TenantManifest {
	const out: TenantManifest = {
		apiVersion: m.apiVersion,
		kind: m.kind,
		metadata: { ...m.metadata },
		tenant: { ...m.tenant },
	};
	if (m.tenant.domainMappings && m.tenant.domainMappings.length === 0) {
		delete out.tenant.domainMappings;
	} else if (m.tenant.domainMappings) {
		out.tenant.domainMappings = [...m.tenant.domainMappings];
	}
	const cfgFields = m.config?.fields ?? {};
	const cfgRefs = m.config?.secretReferences ?? {};
	if (Object.keys(cfgFields).length > 0 || Object.keys(cfgRefs).length > 0) {
		out.config = {};
		if (Object.keys(cfgFields).length > 0) out.config.fields = { ...cfgFields };
		if (Object.keys(cfgRefs).length > 0)
			out.config.secretReferences = { ...cfgRefs };
	}
	if (m.secrets && m.secrets.length > 0) out.secrets = m.secrets;
	if (m.hierarchy && m.hierarchy.length > 0) out.hierarchy = m.hierarchy;
	if (m.roles && m.roles.length > 0) out.roles = m.roles;
	const p = m.providers;
	if (
		p &&
		(p.llm ||
			p.embedding ||
			(p.memoryLayers && Object.keys(p.memoryLayers).length > 0))
	) {
		out.providers = {};
		if (p.llm) out.providers.llm = p.llm;
		if (p.embedding) out.providers.embedding = p.embedding;
		if (p.memoryLayers && Object.keys(p.memoryLayers).length > 0) {
			out.providers.memoryLayers = p.memoryLayers;
		}
	}
	return out;
}

/**
 * Minimal JSON→YAML renderer sufficient for on-screen preview. The
 * submit payload is JSON — this is a UX affordance only.
 */
export function manifestToYaml(v: unknown, indent = 0): string {
	const pad = "  ".repeat(indent);
	if (v === null || v === undefined) return "null";
	if (typeof v === "string") {
		if (v === "" || /[:#\n\-{}[\]&*!|>%@`,]/.test(v) || /^\s|\s$/.test(v)) {
			return JSON.stringify(v);
		}
		return v;
	}
	if (typeof v === "number" || typeof v === "boolean") return String(v);
	if (Array.isArray(v)) {
		if (v.length === 0) return "[]";
		return v
			.map((item) => {
				if (typeof item === "object" && item !== null) {
					const rendered = manifestToYaml(item, indent + 1);
					return `${pad}-\n${rendered}`;
				}
				return `${pad}- ${manifestToYaml(item, indent)}`;
			})
			.join("\n");
	}
	if (typeof v === "object") {
		const entries = Object.entries(v as Record<string, unknown>);
		if (entries.length === 0) return "{}";
		return entries
			.map(([k, val]) => {
				if (val === null || val === undefined) return `${pad}${k}: null`;
				if (typeof val === "object") {
					const isEmpty = Array.isArray(val)
						? val.length === 0
						: Object.keys(val).length === 0;
					if (isEmpty) return `${pad}${k}: ${Array.isArray(val) ? "[]" : "{}"}`;
					return `${pad}${k}:\n${manifestToYaml(val, indent + 1)}`;
				}
				return `${pad}${k}: ${manifestToYaml(val, indent)}`;
			})
			.join("\n");
	}
	return String(v);
}
