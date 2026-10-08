import { api, type Deployment } from "@/lib/api";
import { DeploymentRow } from "@/components/DeploymentRow";
import { Input, Skeleton, isRunning } from "@/components/ui";
import { Search } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

const FILTERS = ["all", "running", "succeeded", "failed"] as const;

export default function Deployments() {
  const [deployments, setDeployments] = useState<Deployment[] | null>(null);
  const [error, setError] = useState("");
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<(typeof FILTERS)[number]>("all");

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
        const q = query.toLowerCase();
        return (
          !q ||
          (d.project_name ?? "").toLowerCase().includes(q) ||
          (d.commit_meta?.message ?? "").toLowerCase().includes(q) ||
          d.commit_sha.toLowerCase().includes(q) ||
          d.branch.toLowerCase().includes(q)
        );
      }),
    [deployments, query, filter],
  );

  return (
    <div className="page-enter mx-auto max-w-5xl p-4 sm:p-8">
      <div className="mb-5 flex items-center justify-between">
        <h1 className="text-xl font-semibold">Deployments</h1>
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
      {error && <p className="mb-4 text-sm text-destructive">{error}</p>}
      <div className="stagger grid gap-2">
        {deployments === null &&
          [0, 1, 2, 3].map((i) => <Skeleton key={i} className="h-[64px]" />)}
        {filtered?.map((d) => <DeploymentRow key={d.id} d={d} showProject />)}
        {filtered !== undefined && filtered?.length === 0 && (
          <div className="rounded-lg border border-dashed border-border p-12 text-center">
            <p className="text-sm text-muted-foreground">
              {deployments?.length === 0
                ? "No deployments yet."
                : `Nothing matches ${query ? `"${query}"` : "this filter"}.`}
            </p>
          </div>
        )}
      </div>
    </div>
  );
}
