import { useState } from "react";
import { Outlet, useLocation, Link } from "react-router-dom";
import { Sidebar } from "@/components/Sidebar";
import { TenantSelector } from "@/components/TenantSelector";
import { SelectTenantBanner } from "@/components/SelectTenantBanner";
import { UserMenu } from "@/components/UserMenu";
import { ChevronRight } from "lucide-react";
import { useTenant } from "@/hooks/useTenant";

const breadcrumbLabels: Record<string, string> = {
	admin: "Dashboard",
	tenants: "Tenants",
	organizations: "Organizations",
	users: "Users",
	knowledge: "Knowledge",
	memory: "Memory",
	governance: "Governance",
	audit: "Audit Log",
	policies: "Policies",
	settings: "Settings",
};

function Breadcrumbs() {
	const location = useLocation();
	const segments = location.pathname
		.replace(/^\/admin\/?/, "")
		.split("/")
		.filter(Boolean);

	if (segments.length === 0) return null;

	return (
		<nav className="flex items-center gap-1 text-sm text-gray-500">
			<Link to="/admin" className="hover:text-gray-700">
				Home
			</Link>
			{segments.map((segment, index) => {
				const path = "/admin/" + segments.slice(0, index + 1).join("/");
				const label = breadcrumbLabels[segment] ?? segment;
				const isLast = index === segments.length - 1;

				return (
					<span key={path} className="flex items-center gap-1">
						<ChevronRight className="h-3.5 w-3.5" />
						{isLast ? (
							<span className="font-medium text-gray-900">{label}</span>
						) : (
							<Link to={path} className="hover:text-gray-700">
								{label}
							</Link>
						)}
					</span>
				);
			})}
		</nav>
	);
}

export function AdminLayout() {
	const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
	const { tenantSourceLabel } = useTenant();

	return (
		<div className="flex h-screen overflow-hidden bg-gray-50">
			<Sidebar
				collapsed={sidebarCollapsed}
				onToggle={() => setSidebarCollapsed(!sidebarCollapsed)}
			/>

			<div className="flex flex-1 flex-col overflow-hidden">
				{/* Header */}
				<header className="flex h-14 items-center justify-between border-b border-gray-200 bg-white px-4">
					<Breadcrumbs />
					<div className="flex items-center gap-3">
						<div className="flex items-center gap-2">
							<TenantSelector />
							{tenantSourceLabel && (
								<span
									title={tenantSourceLabel}
									className="hidden text-xs text-gray-400 md:inline"
								>
									{tenantSourceLabel}
								</span>
							)}
						</div>
						<UserMenu />
					</div>
				</header>

				<SelectTenantBanner />

				{/* Main content */}
				<main className="flex-1 overflow-y-auto p-6">
					<Outlet />
				</main>
			</div>
		</div>
	);
}
