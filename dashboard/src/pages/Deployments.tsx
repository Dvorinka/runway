import { api, type Deployment } from "@/lib/api";
import { DeploymentRow } from "@/components/DeploymentRow";
import { EmptyState, Input, Skeleton, isRunning } from "@/components/ui";
import { Rocket, Search } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

const FILTERS = ["all", "running", "succeeded", "failed"] as const;

export default function Deployments() {
  const [deployments, setDeployments] = useState<Deployment[] | null>(null);
  const [error, setError] = useState("");
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<(typeof FILTERS)[number]>("all");
  const [envs, setEnvs] = useState<Set<string>>(new Set());
  const [projs, setProjs] = useState<Set<string>>(new Set());

  const envChoices = useMemo(
    () => [...new Set(deployments?.map((d) => d.environment_id) ?? [])].sort(),
    [deployments],
  );
  const projChoices = useMemo(
    () => [...new Set(deployments?.map((d) => d.project_name).filter(Boolean) as string[])].sort(),
    [deployments],
  );
  const toggle = (set: Set<string>, v: string, apply: (s: Set<string>) => void) => {
    const next = new Set(set);
    if (next.has(v)) next.delete(v);
    else next.add(v);
    apply(next);
  };

  useEffect(() => {
    const load = () =>
      api
        .deploymentsIndex()
        .then((r) => setDeployments(r.deployments))
        .catch((e) => setError(e.message));
    load();
    const t = setInterval(load, 10_000);
    return () => clearInterval(t);
  }, []);

  const filtered = useMemo(
    () =>
      deployments?.filter((d) => {
        if (
          filter === "running" &&
          !isRunning(d.status, d.conclusion)
        )
          return false;
        if (filter !== "all" && filter !== "running" && d.conclusion !== filter)
          return false;
        if (envs.size > 0 && !envs.has(d.environment_id)) return false;
        if (projs.size > 0 && !projs.has(d.project_name ?? "")) return false;
        const q = query.toLowerCase();
        return (
          !q ||
          (d.project_name ?? "").toLowerCase().includes(q) ||
          (d.commit_meta?.message ?? "").toLowerCase().includes(q) ||
          d.commit_sha.toLowerCase().includes(q) ||
          d.branch.toLowerCase().includes(q)
        );
      }),
    [deployments, query, filter, envs, projs],
  );

  return (
    <div className="page-enter mx-auto max-w-5xl p-4 sm:p-8">
      <div className="mb-5 flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="flex items-center gap-2 text-xl font-semibold">
            Deployments
            <span className="relative flex h-1.5 w-1.5" title="Auto-refreshes every 10s">
              <span className="absolute h-full w-full animate-ping rounded-full bg-emerald-500 opacity-60" />
              <span className="relative h-1.5 w-1.5 rounded-full bg-emerald-500" />
            </span>
          </h1>
          <p className="mt-1 text-sm text-muted-foreground">
            {deployments == null
              ? "Every build and promotion across projects, live."
              : `${deployments.length} total · every build and promotion across projects, live.`}
          </p>
        </div>
        <div className="relative w-56">
          <Search className="absolute left-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            placeholder="Search…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            className="pl-9"
          />
        </div>
      </div>
      <div className="mb-5 flex gap-1 border-b border-border">
        {FILTERS.map((f) => (
          <button
            key={f}
            aria-pressed={filter === f}
            onClick={() => setFilter(f)}
            className={`-mb-px border-b-2 px-3 pb-2 text-sm capitalize transition-colors ${
              filter === f
                ? "border-foreground text-foreground"
                : "border-transparent text-muted-foreground hover:text-foreground"
            }`}
          >
            {f}
          </button>
        ))}
      </div>
      {(envChoices.length > 1 || projChoices.length > 1) && (
        <div className="mb-5 -mt-2 flex flex-wrap items-center gap-1.5 text-xs">
          {projChoices.length > 1 &&
            projChoices.map((p) => (
              <button
                key={p}
                aria-pressed={projs.has(p)}
                onClick={() => toggle(projs, p, setProjs)}
                className={`rounded-full border px-2.5 py-0.5 transition-colors ${
                  projs.has(p)
                    ? "border-foreground/60 bg-accent text-foreground"
                    : "border-border text-muted-foreground hover:text-foreground"
                }`}
              >
                {p}
              </button>
            ))}
          {envChoices.length > 1 &&
            envChoices.map((e) => (
              <button
                key={e}
                aria-pressed={envs.has(e)}
                onClick={() => toggle(envs, e, setEnvs)}
                className={`rounded-full border px-2.5 py-0.5 transition-colors ${
                  envs.has(e)
                    ? "border-foreground/60 bg-accent text-foreground"
                    : "border-border text-muted-foreground hover:text-foreground"
                }`}
              >
                {e === "prod" ? "production" : e}
              </button>
            ))}
        </div>
      )}
      {error && <p className="mb-4 text-sm text-destructive">{error}</p>}
      <div className="stagger grid grid-cols-1 gap-2">
        {deployments === null &&
          [0, 1, 2, 3].map((i) => <Skeleton key={i} className="h-[64px]" />)}
        {filtered?.map((d) => <DeploymentRow key={d.id} d={d} showProject />)}
        {filtered !== undefined && filtered?.length === 0 && (
          <EmptyState
            icon={deployments?.length === 0 ? Rocket : Search}
            title={deployments?.length === 0 ? "No deployments yet" : "No matches"}
            hint={
              deployments?.length === 0
                ? "Link a repo and ship your first deploy."
                : `Nothing matches ${query ? `"${query}"` : "this filter"}.`
            }
          />
        )}
      </div>
    </div>
  );
}
