import { api, type Project } from "@/lib/api";
import { cn } from "@/lib/utils";
import { Bell, FolderGit2, LayoutDashboard, Plus, Rocket, Settings, Users } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";

interface Item {
  id: string;
  label: string;
  hint?: string;
  icon: React.ReactNode;
  to: string;
}

const PAGES: Item[] = [
  {
    id: "nav:overview",
    label: "Overview",
    icon: <LayoutDashboard className="h-4 w-4" />,
    to: "/",
  },
  { id: "nav:new", label: "New project", icon: <Plus className="h-4 w-4" />, to: "/projects" },
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

export function CommandPalette() {
  const nav = useNavigate();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [projects, setProjects] = useState<Project[]>([]);
  const [sel, setSel] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "k") {
        e.preventDefault();
        setOpen((o) => !o);
      } else if (e.key === "Escape") setOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setSel(0);
    api.projects().then((r) => setProjects(r.projects)).catch(() => {});
    const t = setTimeout(() => inputRef.current?.focus(), 10);
    return () => clearTimeout(t);
  }, [open]);

  if (!open) return null;

  const items: Item[] = [
    ...projects.map((p) => ({
      id: p.id,
      label: p.name,
      hint: p.repo_full_name,
      icon: <FolderGit2 className="h-4 w-4" />,
      to: `/projects/${p.id}`,
    })),
    ...PAGES,
  ].filter(
    (i) =>
      !query ||
      i.label.toLowerCase().includes(query.toLowerCase()) ||
      i.hint?.toLowerCase().includes(query.toLowerCase()),
  );

  const go = (to: string) => {
    setOpen(false);
    nav(to);
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/60 pt-[18vh] backdrop-blur-sm"
      onMouseDown={() => setOpen(false)}
    >
      <div
        className="w-full max-w-lg overflow-hidden rounded-lg border border-border bg-card shadow-2xl"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <input
          ref={inputRef}
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
              go(items[sel].to);
            }
          }}
          placeholder="Jump to a project or page…"
          className="w-full border-b border-border bg-transparent px-4 py-3 text-sm placeholder:text-muted-foreground focus:outline-none"
        />
        <div className="max-h-72 overflow-y-auto p-1.5">
          {items.length === 0 && (
            <p className="px-3 py-6 text-center text-xs text-muted-foreground">No matches</p>
          )}
          {items.map((it, i) => (
            <button
              key={it.id}
              onMouseEnter={() => setSel(i)}
              onClick={() => go(it.to)}
              className={cn(
                "flex w-full items-center gap-3 rounded-md px-3 py-2 text-left text-sm",
                i === sel ? "bg-accent" : "",
              )}
            >
              <span className={cn("text-muted-foreground", i === sel && "text-brand")}>{it.icon}</span>
              <span className="flex-1 truncate">{it.label}</span>
              {it.hint && (
                <span className="truncate font-mono text-xs text-muted-foreground">{it.hint}</span>
              )}
            </button>
          ))}
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
