import { api, type Deployment, type Project } from "@/lib/api";
import { DeploymentRow } from "@/components/DeploymentRow";
import { Card, Skeleton, isRunning } from "@/components/ui";
import { ProjectCard } from "@/pages/Projects";
import { useEffect, useState } from "react";
import { Link } from "react-router-dom";

const WEEK = 7 * 24 * 3600 * 1000;

export default function Overview() {
  const [projects, setProjects] = useState<Project[] | null>(null);
  const [deployments, setDeployments] = useState<Deployment[] | null>(null);

  useEffect(() => {
    api.projects().then((r) => setProjects(r.projects)).catch(() => {});
    api.deploymentsIndex().then((r) => setDeployments(r.deployments)).catch(() => {});
  }, []);

  const week = deployments?.filter(
    (d) => Date.now() - new Date(d.created_at).getTime() < WEEK,
  );
  const concluded = week?.filter((d) => d.conclusion);
  const successRate =
    concluded && concluded.length > 0
      ? Math.round(
          (concluded.filter((d) => d.conclusion === "succeeded").length / concluded.length) * 100,
        )
      : null;
  const running = deployments?.filter((d) => isRunning(d.status, d.conclusion)) ?? [];
  const down =
    deployments?.filter(
      (d) =>
        d.conclusion === "succeeded" &&
        ["crashed", "dead", "missing", "paused"].includes(d.computed_status ?? ""),
    ) ?? [];

  const stats: [string, string, string?][] = [
    ["Projects", projects ? String(projects.length) : "—"],
    ["Deploys (7d)", week ? String(week.length) : "—"],
    ["Running", deployments ? String(running.length) : "—", running.length > 0 ? "text-brand" : undefined],
    ["Success rate", successRate !== null ? `${successRate}%` : "—"],
    ["Down", String(down.length), down.length > 0 ? "text-destructive" : undefined],
  ];

  return (
    <div className="page-enter mx-auto max-w-6xl p-4 sm:p-8">
      <h1 className="mb-6 text-xl font-semibold">Overview</h1>
      <div className="mb-8 grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-5">
        {stats.map(([label, value, cls]) => (
          <Card key={label} className="p-4">
            <div className="text-xs text-muted-foreground">{label}</div>
            <div className={`mt-1 font-mono text-2xl tabular-nums ${cls ?? ""}`}>{value}</div>
          </Card>
        ))}
      </div>
      {down.length > 0 && (
        <section className="mb-8">
          <h2 className="mb-3 text-sm font-medium text-destructive">Needs attention</h2>
          <div className="grid gap-2">
            {down.map((d) => (
              <DeploymentRow key={d.id} d={d} showProject />
            ))}
          </div>
        </section>
      )}
      {running.length > 0 && (
        <section className="mb-8">
          <h2 className="mb-3 text-sm font-medium text-muted-foreground">Deploying now</h2>
          <div className="grid gap-2">
            {running.map((d) => (
              <DeploymentRow key={d.id} d={d} showProject />
            ))}
          </div>
        </section>
      )}
      <section className="mb-8">
        <div className="mb-3 flex items-center justify-between">
          <h2 className="text-sm font-medium text-muted-foreground">Projects</h2>
          <Link to="/projects" className="text-xs text-muted-foreground hover:text-foreground">
            View all →
          </Link>
        </div>
        {projects === null ? (
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {[0, 1, 2].map((i) => (
              <Skeleton key={i} className="h-[104px]" />
            ))}
          </div>
        ) : projects.length === 0 ? (
          <div className="rounded-lg border border-dashed border-border p-12 text-center">
            <p className="text-sm text-muted-foreground">No projects yet.</p>
            <Link
              to="/projects"
              className="mt-2 inline-block text-xs text-muted-foreground underline underline-offset-2 hover:text-foreground"
            >
              Create your first project
            </Link>
          </div>
        ) : (
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {projects.slice(0, 6).map((p) => (
              <ProjectCard key={p.id} p={p} />
            ))}
          </div>
        )}
      </section>
      <section>
        <div className="mb-3 flex items-center justify-between">
          <h2 className="text-sm font-medium text-muted-foreground">Recent deployments</h2>
          <Link to="/deployments" className="text-xs text-muted-foreground hover:text-foreground">
            View all →
          </Link>
        </div>
        {deployments === null ? (
          <div className="grid gap-2">
            {[0, 1].map((i) => (
              <Skeleton key={i} className="h-[64px]" />
            ))}
          </div>
        ) : (
          <div className="grid gap-2">
            {deployments.slice(0, 8).map((d) => (
              <DeploymentRow key={d.id} d={d} showProject />
            ))}
            {deployments.length === 0 && (
              <p className="text-sm text-muted-foreground">Nothing deployed yet.</p>
            )}
          </div>
        )}
      </section>
    </div>
  );
}
