import { api, type Me } from "@/lib/api";
import { Avatar } from "@/components/ui";
import { CommandPalette } from "@/components/CommandPalette";
import { cn } from "@/lib/utils";
import {
  Bell,
  ChevronsUpDown,
  FolderKanban,
  LayoutGrid,
  LogOut,
  Menu,
  Rocket,
  Search,
  Settings,
  Users,
  X,
} from "lucide-react";
import { useEffect, useState } from "react";
import { Link, NavLink, useLocation, useNavigate } from "react-router-dom";

function NavItem({
  to,
  icon: Icon,
  children,
  badge,
}: {
  to: string;
  icon: React.ComponentType<{ className?: string }>;
  children: React.ReactNode;
  badge?: number;
}) {
  return (
    <NavLink
      to={to}
      end={to === "/"}
      className={({ isActive }) =>
        cn(
          "flex h-8 items-center gap-2.5 rounded-md px-2 text-sm transition-colors",
          isActive
            ? "bg-accent text-foreground"
            : "text-muted-foreground hover:bg-accent/60 hover:text-foreground",
        )
      }
    >
      <Icon className="h-4 w-4 shrink-0" />
      <span className="min-w-0 flex-1 truncate">{children}</span>
      {badge !== undefined && badge > 0 && (
        <span className="rounded-full bg-brand px-1.5 text-[11px] leading-4 font-medium text-white">
          {badge}
        </span>
      )}
    </NavLink>
  );
}

function RailNav({ unread, onNavigate }: { unread: number; onNavigate?: () => void }) {
  return (
    <nav className="flex-1 space-y-0.5 overflow-y-auto px-3 py-2" onClick={onNavigate}>
      <NavItem to="/" icon={LayoutGrid}>Overview</NavItem>
      <NavItem to="/projects" icon={FolderKanban}>Projects</NavItem>
      <NavItem to="/deployments" icon={Rocket}>Deployments</NavItem>
      <NavItem to="/notifications" icon={Bell} badge={unread}>Notifications</NavItem>
      <NavItem to="/teams" icon={Users}>Teams</NavItem>
      <NavItem to="/settings" icon={Settings}>Settings</NavItem>
    </nav>
  );
}

function FindButton() {
  return (
    <div className="px-3 pb-2">
      <button
        type="button"
        onClick={() => window.dispatchEvent(new Event("runway:palette"))}
        className="flex h-8 w-full items-center gap-2 rounded-md border border-border px-2.5 text-xs text-muted-foreground transition-colors hover:border-muted-foreground/40 hover:text-foreground"
        title="Jump to projects, deployments, settings…"
      >
        <Search className="h-3.5 w-3.5" />
        <span className="flex-1 text-left">Find</span>
        <kbd className="rounded border border-border bg-muted/50 px-1 font-mono text-[10px]">
          ⌘K
        </kbd>
        <kbd className="rounded border border-border bg-muted/50 px-1 font-mono text-[10px]">
          F
        </kbd>
      </button>
    </div>
  );
}

function TopBar() {
  const { pathname } = useLocation();
  const nav = useNavigate();
  const projectId = pathname.match(/^\/projects\/([^/]+)/)?.[1];
  const deploymentId = pathname.match(/^\/deployments\/([^/]+)/)?.[1];
  const [projectName, setProjectName] = useState("");
  const [depLabel, setDepLabel] = useState("");
  useEffect(() => {
    setProjectName("");
    if (projectId) api.project(projectId).then((p) => setProjectName(p.name)).catch(() => {});
  }, [projectId]);
  useEffect(() => {
    setDepLabel("");
    if (deploymentId)
      api.deployment(deploymentId).then((d) => setDepLabel(d.commit_sha.slice(0, 7))).catch(() => {});
  }, [deploymentId]);

  const parent = projectId
    ? { label: "Projects", to: "/projects" }
    : deploymentId
      ? { label: "Deployments", to: "/deployments" }
      : null;

  const title = projectId
    ? projectName || "Project"
    : deploymentId
      ? depLabel || deploymentId.slice(0, 7)
      : pathname === "/"
        ? "Overview"
        : pathname.startsWith("/projects")
          ? "Projects"
          : pathname.startsWith("/deployments")
            ? "Deployments"
            : pathname.startsWith("/notifications")
              ? "Notifications"
              : pathname.startsWith("/teams")
                ? "Teams"
                : pathname.startsWith("/settings")
                  ? "Settings"
                  : "Runway";

  return (
    <div className="sticky top-0 z-30 hidden h-11 items-center justify-between border-b border-border bg-background/90 px-6 backdrop-blur lg:flex">
      <span className="w-40 truncate text-xs text-muted-foreground">
        {parent && (
          <Link to={parent.to} className="transition-colors hover:text-foreground">
            {parent.label}
          </Link>
        )}
      </span>
      <span className="truncate text-sm font-medium">{title}</span>
      <div className="flex w-40 justify-end">
        <button
          type="button"
          onClick={() => nav("/projects?new=1")}
          className="rounded-md border border-border px-2.5 py-1 text-xs text-muted-foreground transition-colors hover:text-foreground"
        >
          New project
        </button>
      </div>
    </div>
  );
}

function AccountRow({ me }: { me: Me }) {
  const nav = useNavigate();
  return (
    <div className="border-t border-border p-3">
      <div className="flex items-center gap-2">
        <Link
          to="/teams"
          className="flex min-w-0 flex-1 items-center gap-2 rounded-md px-1.5 py-1.5 transition-colors hover:bg-accent/60"
          title="Teams"
        >
          <Avatar
            kind="user"
            id={me.id}
            name={me.name ?? me.username ?? me.email}
            hasAvatar={me.has_avatar}
            className="h-6 w-6 text-[10px]"
          />
          <span className="min-w-0 flex-1 truncate text-sm">
            {me.name ?? me.username ?? me.email}
          </span>
          <ChevronsUpDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        </Link>
        <button
          type="button"
          onClick={() => api.logout().then(() => nav(0)).catch(() => nav(0))}
          className="rounded-md p-1.5 text-muted-foreground transition-colors hover:bg-accent/60 hover:text-foreground"
          title="Sign out"
        >
          <LogOut className="h-4 w-4" />
        </button>
      </div>
    </div>
  );
}

export default function Layout({ me, children }: { me: Me; children: React.ReactNode }) {
  const [unread, setUnread] = useState(0);
  const [menu, setMenu] = useState(false);
  const { pathname } = useLocation();
  useEffect(() => {
    const poll = () =>
      api
        .notifications()
        .then((r) => setUnread(r.unread))
        .catch(() => {});
    poll();
    const t = setInterval(poll, 30_000);
    return () => clearInterval(t);
  }, []);

  useEffect(() => setMenu(false), [pathname]);

  const rail = (
    <>
      <Link to="/" className="flex h-14 items-center gap-2.5 px-4 font-semibold tracking-tight">
        <img src="/runway-mark-white.svg" alt="" className="h-5 w-5" />
        Runway
      </Link>
      <FindButton />
      <RailNav unread={unread} />
      <AccountRow me={me} />
    </>
  );

  return (
    <div className="flex min-h-screen">
      <aside className="sticky top-0 hidden h-screen w-60 shrink-0 flex-col border-r border-border lg:flex">
        {rail}
      </aside>

      <div className="min-w-0 flex-1">
        {/* Mobile top bar — same hierarchy, re-housed */}
        <header className="sticky top-0 z-40 flex h-14 items-center justify-between border-b border-border bg-background px-4 lg:hidden">
          <Link to="/" className="flex items-center gap-2.5 font-semibold tracking-tight">
            <img src="/runway-mark-white.svg" alt="" className="h-5 w-5" />
            Runway
          </Link>
          <div className="flex items-center gap-1">
            <button
              type="button"
              onClick={() => window.dispatchEvent(new Event("runway:palette"))}
              className="rounded-md p-2 text-muted-foreground transition-colors hover:bg-accent/60 hover:text-foreground"
              title="Find"
            >
              <Search className="h-4 w-4" />
            </button>
            <button
              type="button"
              onClick={() => setMenu((m) => !m)}
              className="rounded-md p-2 text-muted-foreground transition-colors hover:bg-accent/60 hover:text-foreground"
              title="Menu"
            >
              {menu ? <X className="h-4 w-4" /> : <Menu className="h-4 w-4" />}
            </button>
          </div>
        </header>

        {menu && (
          <>
            <div
              className="fixed inset-0 z-40 bg-black/60 backdrop-blur-sm lg:hidden"
              onClick={() => setMenu(false)}
            />
            <aside className="fixed inset-y-0 left-0 z-50 flex w-60 flex-col border-r border-border bg-background lg:hidden">
              {rail}
            </aside>
          </>
        )}

        <TopBar />
        <main className="min-w-0">{children}</main>
      </div>
      <CommandPalette />
    </div>
  );
}
