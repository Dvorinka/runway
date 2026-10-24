import { api, type Deployment, type Project } from "@/lib/api";
import { cn } from "@/lib/utils";
import { Bell, Copy, ExternalLink, FolderGit2, LayoutDashboard, Plus, Rocket, Settings, Users } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useLocation, useNavigate } from "react-router-dom";

interface Item {
  id: string;
  label: string;
  hint?: string;
  icon: React.ReactNode;
  to?: string;
  run?: () => void;
}

const ACTIONS: Item[] = [
  { id: "act:new", label: "New project", hint: "create", icon: <Plus className="h-4 w-4" />, to: "/projects?new=1" },
];

const PAGES: Item[] = [
  {
    id: "nav:overview",
    label: "Overview",
    icon: <LayoutDashboard className="h-4 w-4" />,
    to: "/",
  },
  {
    id: "nav:deployments",
    label: "Deployments",
    icon: <Rocket className="h-4 w-4" />,
    to: "/deployments",
  },
  { id: "nav:teams", label: "Teams", icon: <Users className="h-4 w-4" />, to: "/teams" },
  {
    id: "nav:notifications",
    label: "Notifications",
    icon: <Bell className="h-4 w-4" />,
    to: "/notifications",
  },
  {
    id: "nav:settings",
    label: "Settings",
    icon: <Settings className="h-4 w-4" />,
    to: "/settings",
  },
];

const PROJECT_TABS: [string, string][] = [
  ["Deployments", "deployments"],
  ["Logs", "logs"],
  ["Environment", "environment"],
  ["Analytics", "analytics"],
  ["Speed", "speed"],
  ["Cron", "cron"],
  ["Redirects", "redirects"],
  ["Domains", "domains"],
  ["Webhooks", "webhooks"],
  ["Settings", "settings"],
];

export function CommandPalette() {
  const nav = useNavigate();
  const { pathname } = useLocation();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [projects, setProjects] = useState<Project[]>([]);
  const [deployments, setDeployments] = useState<Deployment[]>([]);
  const [sel, setSel] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement;
      const typing =
        el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable;
      if ((e.metaKey || e.ctrlKey) && e.key === "k") {
        e.preventDefault();
        setOpen((o) => !o);
      } else if (e.key === "f" && !typing && !e.metaKey && !e.ctrlKey && !e.altKey) {
        e.preventDefault();
        setOpen(true);
      } else if (e.key === "Escape") setOpen(false);
    };
    const onOpen = () => setOpen(true);
    window.addEventListener("keydown", onKey);
    window.addEventListener("runway:palette", onOpen);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("runway:palette", onOpen);
    };
  }, []);

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setSel(0);
    api.projects().then((r) => setProjects(r.projects)).catch(() => {});
    api.deploymentsIndex().then((r) => setDeployments(r.deployments)).catch(() => {});
    const t = setTimeout(() => inputRef.current?.focus(), 10);
    return () => clearTimeout(t);
  }, [open]);

  if (!open) return null;

  // Subsequence fuzzy match — "dmo" still finds "demo-app".
  const fuzzy = (q: string, s: string) => {
    let i = 0;
    for (const c of s.toLowerCase()) if (c === q[i]) i++;
    return i >= q.length;
  };
  const match = (i: Item) =>
    !query ||
    fuzzy(query, i.label) ||
    (i.hint ? fuzzy(query, i.hint) : false);

  const scopedId = pathname.match(/^\/projects\/([^/]+)/)?.[1];
  const scopedProject = projects.find((p) => p.id === scopedId);
  const prodUrl = scopedProject?.url ?? null;
  const scopedActions: Item[] = (
    scopedProject
      ? [
          ...(prodUrl
            ? [
                {
                  id: "scope:open-prod",
                  label: "Open production site",
                  hint: prodUrl.replace(/^https?:\/\//, ""),
                  icon: <ExternalLink className="h-4 w-4" />,
                  run: () =>
                    window.open(
                      prodUrl.startsWith("http") ? prodUrl : `https://${prodUrl}`,
                      "_blank",
                    ),
                },
              ]
            : []),
          {
            id: "scope:copy-url",
            label: "Copy production URL",
            hint: prodUrl ? "copy" : "no URL",
            icon: <Copy className="h-4 w-4" />,
            run: () => {
              if (prodUrl)
                navigator.clipboard.writeText(
                  prodUrl.startsWith("http") ? prodUrl : `https://${prodUrl}`,
                );
            },
          },
          {
            id: "scope:copy-id",
            label: "Copy project ID",
            hint: "copy",
            icon: <Copy className="h-4 w-4" />,
            run: () => navigator.clipboard.writeText(scopedProject.id),
          },
        ]
      : []
  ).filter(match);
  const scopedItems: Item[] = scopedProject
    ? [
        ...PROJECT_TABS.map(([label, tab]) => ({
          id: `scope:${tab}`,
          label,
          hint: scopedProject.name,
          icon: <FolderGit2 className="h-4 w-4" />,
          to: tab === "deployments" ? `/projects/${scopedId}` : `/projects/${scopedId}?tab=${tab}`,
        })),
        ...scopedActions,
      ].filter(match)
    : [];
  const projectItems: Item[] = projects
    .filter((p) => p.id !== scopedId)
    .map((p) => ({
      id: p.id,
      label: p.name,
      hint: p.repo_full_name,
      icon: <FolderGit2 className="h-4 w-4" />,
      to: `/projects/${p.id}`,
    }))
    .filter(match);
  const deploymentItems: Item[] = deployments
    .map((d) => ({
      id: `dep:${d.id}`,
      label: `${d.commit_sha.slice(0, 7)} — ${d.project_name ?? "deployment"}`,
      hint: d.environment_id === "prod" ? "production" : d.environment_id,
      icon: <Rocket className="h-4 w-4" />,
      to: `/deployments/${d.id}`,
    }))
    .filter((it) => !query || fuzzy(query, it.label) || fuzzy(query, it.id));
  const actionItems = ACTIONS.filter(match);
  const pageItems = PAGES.filter(match);
  const items = [...scopedItems, ...projectItems, ...deploymentItems, ...actionItems, ...pageItems];

  const go = (it: Item) => {
    setOpen(false);
    if (it.run) it.run();
    else if (it.to) nav(it.to);
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/60 pt-[18vh] backdrop-blur-sm"
      onMouseDown={() => setOpen(false)}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
        className="w-full max-w-lg overflow-hidden rounded-lg border border-border bg-card shadow-2xl"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <input
          ref={inputRef}
          role="combobox"
          aria-expanded="true"
          aria-label="Jump to a project or page"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setSel(0);
          }}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") {
              e.preventDefault();
              setSel((s) => Math.min(s + 1, items.length - 1));
            } else if (e.key === "ArrowUp") {
              e.preventDefault();
              setSel((s) => Math.max(s - 1, 0));
            } else if (e.key === "Enter" && items[sel]) {
              go(items[sel]);
            }
          }}
          placeholder="Jump to a project or page…"
          className="w-full border-b border-border bg-transparent px-4 py-3 text-sm placeholder:text-muted-foreground focus:outline-none"
        />
        <div role="listbox" className="max-h-72 overflow-y-auto p-1.5">
          {items.length === 0 && (
            <p className="px-3 py-6 text-center text-xs text-muted-foreground">No matches</p>
          )}
          {(
            [
              [scopedProject?.name ?? "", scopedItems],
              ["Projects", projectItems],
              ["Deployments", deploymentItems],
              ["Actions", actionItems],
              ["Pages", pageItems],
            ] as [string, Item[]][]
          ).map(([group, groupItems]) => {
            if (groupItems.length === 0) return null;
            return (
              <div key={group}>
                <div className="px-3 pb-1 pt-2 text-[11px] font-medium text-muted-foreground/70">
                  {group}
                </div>
                {groupItems.map((it) => {
                  const i = items.indexOf(it);
                  return (
                    <button
                      key={it.id}
                      role="option"
                      aria-selected={i === sel}
                      onMouseEnter={() => setSel(i)}
                      onClick={() => go(it)}
                      className={cn(
                        "flex w-full items-center gap-3 rounded-md px-3 py-2 text-left text-sm",
                        i === sel ? "bg-accent" : "",
                      )}
                    >
                      <span className={cn("text-muted-foreground", i === sel && "text-foreground")}>{it.icon}</span>
                      <span className="flex-1 truncate">{it.label}</span>
                      {it.hint && (
                        <span className="truncate font-mono text-xs text-muted-foreground">{it.hint}</span>
                      )}
                    </button>
                  );
                })}
              </div>
            );
          })}
        </div>
        <div className="flex items-center gap-4 border-t border-border px-4 py-2 text-[11px] text-muted-foreground">
          <span>↑↓ navigate</span>
          <span>↵ open</span>
          <span>esc close</span>
        </div>
      </div>
    </div>
  );
}
