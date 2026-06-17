/**
 * Wizard step panels. Each step is a pure function of the current
 * in-progress manifest and an onChange callback.
 *
 * §12.2–§12.6 — harden-tenant-provisioning.
 */
import { useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import type {
	TenantManifest,
	ManifestSecretReference,
	SecretReferenceKind,
	ManifestUnitNode,
	ManifestRoleAssignment,
	ManifestProvider,
} from "@/api/tenant-manifest";

const ROLES = [
	"TenantAdmin",
	"Admin",
	"Architect",
	"TechLead",
	"Developer",
	"Viewer",
	"Agent",
];

const UNIT_TYPES: ManifestUnitNode["unitType"][] = [
	"Organization",
	"Team",
	"Project",
];

const SECRET_KINDS: SecretReferenceKind[] = [
	"env",
	"file",
	"k8s",
	"postgres",
	"inline",
];

const inputCls =
	"mt-1 block w-full rounded-md border border-gray-300 px-3 py-2 text-sm shadow-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-700 dark:text-gray-100";
const labelCls = "block text-sm font-medium text-gray-700 dark:text-gray-300";

export interface StepProps {
	manifest: TenantManifest;
	onChange: (m: TenantManifest) => void;
}

// ---------------------------------------------------------------- Step 1
// §12.2 — tenant identity.
export function IdentityStep({ manifest, onChange }: StepProps) {
	const [domain, setDomain] = useState("");
	const mappings = manifest.tenant.domainMappings ?? [];

	return (
		<div className="space-y-4" data-testid="wizard-step-identity">
			<div>
				<label className={labelCls} htmlFor="wiz-slug">
					Slug *
				</label>
				<input
					id="wiz-slug"
					type="text"
					required
					pattern="[a-z0-9-]+"
					value={manifest.tenant.slug}
					onChange={(e) =>
						onChange({
							...manifest,
							tenant: { ...manifest.tenant, slug: e.target.value },
						})
					}
					placeholder="acme"
					className={inputCls}
				/>
				<p className="mt-1 text-xs text-gray-500">
					Lowercase letters, digits, hyphens.
				</p>
			</div>
			<div>
				<label className={labelCls} htmlFor="wiz-name">
					Display name *
				</label>
				<input
					id="wiz-name"
					type="text"
					required
					value={manifest.tenant.name}
					onChange={(e) =>
						onChange({
							...manifest,
							tenant: { ...manifest.tenant, name: e.target.value },
						})
					}
					placeholder="Acme Corp"
					className={inputCls}
				/>
			</div>
			<div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
				<div>
					<label className={labelCls} htmlFor="wiz-account-ref">
						Account
					</label>
					<input
						id="wiz-account-ref"
						type="text"
						value={manifest.tenant.accountRef ?? ""}
						onChange={(e) =>
							onChange({
								...manifest,
								tenant: {
									...manifest.tenant,
									accountRef: e.target.value || undefined,
								},
							})
						}
						placeholder="acme"
						className={inputCls}
					/>
					<p className="mt-1 text-xs text-gray-500">
						Optional account slug or UUID.
					</p>
				</div>
				<div>
					<label className={labelCls} htmlFor="wiz-environment">
						Environment
					</label>
					<input
						id="wiz-environment"
						type="text"
						value={manifest.tenant.environment ?? ""}
						onChange={(e) =>
							onChange({
								...manifest,
								tenant: {
									...manifest.tenant,
									environment: e.target.value || undefined,
								},
							})
						}
						placeholder="prod"
						className={inputCls}
					/>
					<p className="mt-1 text-xs text-gray-500">
						Optional label such as dev, test, staging, or prod.
					</p>
				</div>
			</div>
			<div>
				<label className={labelCls}>Domain mappings</label>
				<div className="mt-1 flex gap-2">
					<input
						type="text"
						value={domain}
						onChange={(e) => setDomain(e.target.value)}
						placeholder="acme.example.com"
						className={inputCls}
						aria-label="New domain mapping"
					/>
					<button
						type="button"
						disabled={!domain.trim()}
						onClick={() => {
							onChange({
								...manifest,
								tenant: {
									...manifest.tenant,
									domainMappings: [...mappings, domain.trim()],
								},
							});
							setDomain("");
						}}
						className="inline-flex items-center gap-1 rounded-md bg-gray-200 px-3 py-2 text-sm hover:bg-gray-300 disabled:opacity-50 dark:bg-gray-700 dark:hover:bg-gray-600"
					>
						<Plus className="h-4 w-4" /> Add
					</button>
				</div>
				{mappings.length > 0 && (
					<ul className="mt-2 space-y-1">
						{mappings.map((d, i) => (
							<li
								key={`${d}-${i}`}
								className="flex items-center justify-between rounded-md bg-gray-50 px-3 py-1 text-sm dark:bg-gray-700"
							>
								<span className="font-mono">{d}</span>
								<button
									type="button"
									onClick={() =>
										onChange({
											...manifest,
											tenant: {
												...manifest.tenant,
												domainMappings: mappings.filter((_, j) => j !== i),
											},
										})
									}
									className="text-red-600 hover:text-red-700"
									aria-label={`Remove ${d}`}
								>
									<Trash2 className="h-4 w-4" />
								</button>
							</li>
						))}
					</ul>
				)}
			</div>
		</div>
	);
}

// ---------------------------------------------------------------- Step 2
// §12.3 — secret references. The picker is structural only; the
// `GET /admin/secret-sources` backend endpoint is tracked as a follow-up
// (see tasks.md §12.3 note). Until it lands, the operator types names
// directly and the UI validates kind-specific required fields.
export function SecretsStep({ manifest, onChange }: StepProps) {
	const refs = manifest.config?.secretReferences ?? {};
	const [name, setName] = useState("");
	const [kind, setKind] = useState<SecretReferenceKind>("env");

	const addRef = () => {
		if (!name.trim()) return;
		const key = name.trim();
		onChange({
			...manifest,
			config: {
				...(manifest.config ?? {}),
				secretReferences: {
					...refs,
					[key]: { logicalName: key, kind } as ManifestSecretReference,
				},
			},
		});
		setName("");
	};

	const updateRef = (key: string, patch: Partial<ManifestSecretReference>) => {
		onChange({
			...manifest,
			config: {
				...(manifest.config ?? {}),
				secretReferences: { ...refs, [key]: { ...refs[key], ...patch } },
			},
		});
	};

	const removeRef = (key: string) => {
		const next = { ...refs };
		delete next[key];
		onChange({
			...manifest,
			config: { ...(manifest.config ?? {}), secretReferences: next },
		});
	};

	const entries = Object.entries(refs);

	return (
		<div className="space-y-4" data-testid="wizard-step-secrets">
			<p className="text-sm text-gray-600 dark:text-gray-400">
				Declare secret references by logical name. Plaintext values never leave
				the browser unless the server explicitly allows inline secrets.
			</p>
			<div className="flex gap-2">
				<input
					type="text"
					value={name}
					onChange={(e) => setName(e.target.value)}
					placeholder="db.password"
					className={inputCls}
					aria-label="New reference name"
				/>
				<select
					value={kind}
					onChange={(e) => setKind(e.target.value as SecretReferenceKind)}
					className={inputCls + " max-w-[10rem]"}
					aria-label="New reference kind"
				>
					{SECRET_KINDS.map((k) => (
						<option key={k} value={k}>
							{k}
						</option>
					))}
				</select>
				<button
					type="button"
					onClick={addRef}
					disabled={!name.trim()}
					className="inline-flex items-center gap-1 rounded-md bg-blue-600 px-3 py-2 text-sm text-white hover:bg-blue-700 disabled:opacity-50"
				>
					<Plus className="h-4 w-4" /> Add
				</button>
			</div>
			{entries.length === 0 ? (
				<p className="text-sm text-gray-500 italic">No references declared.</p>
			) : (
				<ul className="space-y-3">
					{entries.map(([key, secretRef]) => (
						<li
							key={key}
							className="rounded-md border border-gray-200 p-3 dark:border-gray-700"
						>
							<div className="mb-2 flex items-center justify-between">
								<div className="flex items-center gap-2">
									<span className="font-mono text-sm font-semibold">{key}</span>
									<span className="rounded bg-gray-100 px-2 py-0.5 text-xs dark:bg-gray-700">
										{secretRef.kind}
									</span>
								</div>
								<button
									type="button"
									onClick={() => removeRef(key)}
									className="text-red-600 hover:text-red-700"
									aria-label={`Remove reference ${key}`}
								>
									<Trash2 className="h-4 w-4" />
								</button>
							</div>
							<SecretRefFields
								refKey={key}
								secretRef={secretRef}
								onPatch={(p) => updateRef(key, p)}
							/>
						</li>
					))}
				</ul>
			)}
		</div>
	);
}

function SecretRefFields({
	refKey,
	secretRef,
	onPatch,
}: {
	refKey: string;
	secretRef: ManifestSecretReference;
	onPatch: (p: Partial<ManifestSecretReference>) => void;
}) {
	switch (secretRef.kind) {
		case "env":
			return (
				<input
					type="text"
					placeholder="ENV_VAR_NAME"
					value={secretRef.var ?? ""}
					onChange={(e) => onPatch({ var: e.target.value })}
					className={inputCls}
					aria-label={`${refKey} env variable`}
				/>
			);
		case "file":
			return (
				<input
					type="text"
					placeholder="/etc/aeterna/secrets/name"
					value={secretRef.path ?? ""}
					onChange={(e) => onPatch({ path: e.target.value })}
					className={inputCls}
					aria-label={`${refKey} file path`}
				/>
			);
		case "k8s":
			return (
				<div className="grid grid-cols-3 gap-2">
					<input
						type="text"
						placeholder="secret name"
						value={secretRef.name ?? ""}
						onChange={(e) => onPatch({ name: e.target.value })}
						className={inputCls}
						aria-label={`${refKey} k8s name`}
					/>
					<input
						type="text"
						placeholder="key"
						value={secretRef.key ?? ""}
						onChange={(e) => onPatch({ key: e.target.value })}
						className={inputCls}
						aria-label={`${refKey} k8s key`}
					/>
					<input
						type="text"
						placeholder="namespace (optional)"
						value={secretRef.namespace ?? ""}
						onChange={(e) => onPatch({ namespace: e.target.value })}
						className={inputCls}
						aria-label={`${refKey} k8s namespace`}
					/>
				</div>
			);
		case "postgres":
			return (
				<div className="grid grid-cols-2 gap-2">
					<input
						type="text"
						placeholder="secret id"
						value={secretRef.secretId ?? ""}
						onChange={(e) => onPatch({ secretId: e.target.value })}
						className={inputCls}
						aria-label={`${refKey} pg secret id`}
					/>
					<input
						type="text"
						placeholder="label"
						value={secretRef.label ?? ""}
						onChange={(e) => onPatch({ label: e.target.value })}
						className={inputCls}
						aria-label={`${refKey} pg label`}
					/>
				</div>
			);
		case "inline":
			return (
				<>
					<input
						type="password"
						placeholder="plaintext value (dev only)"
						value={secretRef.inline ?? ""}
						onChange={(e) => onPatch({ inline: e.target.value })}
						className={inputCls}
						aria-label={`${refKey} inline value`}
					/>
					<p className="mt-1 text-xs text-amber-600">
						Inline plaintext is rejected unless the server has
						`provisioning.allowInlineSecret = true` AND the submit adds
						`?allowInline=true`.
					</p>
				</>
			);
	}
}

// ---------------------------------------------------------------- Step 3
// §12.4 — initial hierarchy (flat first-pass; tree edit is a follow-up).
export function HierarchyStep({ manifest, onChange }: StepProps) {
	const hierarchy = manifest.hierarchy ?? [];
	const [name, setName] = useState("");
	const [unitType, setUnitType] =
		useState<ManifestUnitNode["unitType"]>("Organization");

	const add = () => {
		if (!name.trim()) return;
		onChange({
			...manifest,
			hierarchy: [...hierarchy, { name: name.trim(), unitType }],
		});
		setName("");
	};

	const remove = (i: number) =>
		onChange({ ...manifest, hierarchy: hierarchy.filter((_, j) => j !== i) });

	return (
		<div className="space-y-4" data-testid="wizard-step-hierarchy">
			<p className="text-sm text-gray-600 dark:text-gray-400">
				Declare tenant-root organizations, teams, or projects. In practice most
				manifests should start with organizations; nested children can be edited
				from the tenant page after creation.
			</p>
			<div className="flex gap-2">
				<input
					type="text"
					placeholder="Unit name"
					value={name}
					onChange={(e) => setName(e.target.value)}
					className={inputCls}
					aria-label="Unit name"
				/>
				<select
					value={unitType}
					onChange={(e) =>
						setUnitType(e.target.value as ManifestUnitNode["unitType"])
					}
					className={inputCls + " max-w-[10rem]"}
					aria-label="Unit type"
				>
					{UNIT_TYPES.map((t) => (
						<option key={t} value={t}>
							{t}
						</option>
					))}
				</select>
				<button
					type="button"
					onClick={add}
					disabled={!name.trim()}
					className="inline-flex items-center gap-1 rounded-md bg-blue-600 px-3 py-2 text-sm text-white hover:bg-blue-700 disabled:opacity-50"
				>
					<Plus className="h-4 w-4" /> Add
				</button>
			</div>
			{hierarchy.length === 0 ? (
				<p className="text-sm text-gray-500 italic">No units declared.</p>
			) : (
				<ul className="space-y-1">
					{hierarchy.map((u, i) => (
						<li
							key={`${u.name}-${i}`}
							className="flex items-center justify-between rounded-md bg-gray-50 px-3 py-1 text-sm dark:bg-gray-700"
						>
							<span>
								<strong>{u.name}</strong>{" "}
								<span className="text-xs text-gray-500">[{u.unitType}]</span>
							</span>
							<button
								type="button"
								onClick={() => remove(i)}
								className="text-red-600 hover:text-red-700"
								aria-label={`Remove ${u.name}`}
							>
								<Trash2 className="h-4 w-4" />
							</button>
						</li>
					))}
				</ul>
			)}
		</div>
	);
}

// ---------------------------------------------------------------- Step 4
// §12.5 — role assignments.
export function RolesStep({ manifest, onChange }: StepProps) {
	const roles = manifest.roles ?? [];
	const [email, setEmail] = useState("");
	const [role, setRole] = useState("TenantAdmin");

	const add = () => {
		if (!email.trim()) return;
		onChange({
			...manifest,
			roles: [
				...roles,
				{ email: email.trim(), role } as ManifestRoleAssignment,
			],
		});
		setEmail("");
	};

	const remove = (i: number) =>
		onChange({ ...manifest, roles: roles.filter((_, j) => j !== i) });

	return (
		<div className="space-y-4" data-testid="wizard-step-roles">
			<p className="text-sm text-gray-600 dark:text-gray-400">
				Grant users a role at tenant scope. Per-unit assignments can be added
				later.
			</p>
			<div className="flex gap-2">
				<input
					type="email"
					placeholder="user@example.com"
					value={email}
					onChange={(e) => setEmail(e.target.value)}
					className={inputCls}
					aria-label="User email"
				/>
				<select
					value={role}
					onChange={(e) => setRole(e.target.value)}
					className={inputCls + " max-w-[12rem]"}
					aria-label="Role"
				>
					{ROLES.map((r) => (
						<option key={r} value={r}>
							{r}
						</option>
					))}
				</select>
				<button
					type="button"
					onClick={add}
					disabled={!email.trim()}
					className="inline-flex items-center gap-1 rounded-md bg-blue-600 px-3 py-2 text-sm text-white hover:bg-blue-700 disabled:opacity-50"
				>
					<Plus className="h-4 w-4" /> Add
				</button>
			</div>
			{roles.length === 0 ? (
				<p className="text-sm text-gray-500 italic">No role assignments.</p>
			) : (
				<ul className="space-y-1">
					{roles.map((r, i) => (
						<li
							key={`${r.email ?? r.userId}-${i}`}
							className="flex items-center justify-between rounded-md bg-gray-50 px-3 py-1 text-sm dark:bg-gray-700"
						>
							<span className="font-mono">{r.email ?? r.userId}</span>
							<div className="flex items-center gap-2">
								<span className="rounded bg-blue-100 px-2 py-0.5 text-xs text-blue-800 dark:bg-blue-900 dark:text-blue-200">
									{r.role}
								</span>
								<button
									type="button"
									onClick={() => remove(i)}
									className="text-red-600 hover:text-red-700"
									aria-label={`Remove ${r.email ?? r.userId}`}
								>
									<Trash2 className="h-4 w-4" />
								</button>
							</div>
						</li>
					))}
				</ul>
			)}
		</div>
	);
}

// ---------------------------------------------------------------- Step 5
// §12.6 — provider declarations.
export function ProvidersStep({ manifest, onChange }: StepProps) {
	const providers = manifest.providers ?? {};

	const setLlm = (p: ManifestProvider | undefined) =>
		onChange({ ...manifest, providers: { ...providers, llm: p } });
	const setEmb = (p: ManifestProvider | undefined) =>
		onChange({ ...manifest, providers: { ...providers, embedding: p } });

	return (
		<div className="space-y-6" data-testid="wizard-step-providers">
			<p className="text-sm text-gray-600 dark:text-gray-400">
				Declare provider kinds for LLM and embedding. Memory layers are
				configured on the tenant page.
			</p>
			<ProviderBlock
				label="LLM provider"
				provider={providers.llm}
				onChange={setLlm}
				kinds={["openai", "anthropic", "google", "bedrock", "local"]}
				idPrefix="llm"
			/>
			<ProviderBlock
				label="Embedding provider"
				provider={providers.embedding}
				onChange={setEmb}
				kinds={["openai", "google", "bedrock", "local"]}
				idPrefix="emb"
			/>
		</div>
	);
}

function ProviderBlock({
	label,
	provider,
	onChange,
	kinds,
	idPrefix,
}: {
	label: string;
	provider: ManifestProvider | undefined;
	onChange: (p: ManifestProvider | undefined) => void;
	kinds: string[];
	idPrefix: string;
}) {
	const enabled = !!provider;
	return (
		<div className="rounded-md border border-gray-200 p-3 dark:border-gray-700">
			<label className="flex items-center gap-2 text-sm font-medium">
				<input
					type="checkbox"
					checked={enabled}
					onChange={(e) =>
						onChange(e.target.checked ? { kind: kinds[0] } : undefined)
					}
					aria-label={label}
				/>
				{label}
			</label>
			{enabled && provider && (
				<div className="mt-3 grid grid-cols-2 gap-2">
					<div>
						<label className={labelCls} htmlFor={`${idPrefix}-kind`}>
							Kind
						</label>
						<select
							id={`${idPrefix}-kind`}
							value={provider.kind}
							onChange={(e) => onChange({ ...provider, kind: e.target.value })}
							className={inputCls}
						>
							{kinds.map((k) => (
								<option key={k} value={k}>
									{k}
								</option>
							))}
						</select>
					</div>
					<div>
						<label className={labelCls} htmlFor={`${idPrefix}-model`}>
							Model
						</label>
						<input
							id={`${idPrefix}-model`}
							type="text"
							value={provider.model ?? ""}
							onChange={(e) => onChange({ ...provider, model: e.target.value })}
							placeholder="gpt-4o / claude-3-5-sonnet / ..."
							className={inputCls}
						/>
					</div>
				</div>
			)}
		</div>
	);
}
