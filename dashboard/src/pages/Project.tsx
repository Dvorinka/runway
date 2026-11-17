import { api, type Project } from "@/lib/api";
import { Avatar, EmptyState } from "@/components/ui";
import { Analytics } from "@/pages/project/analytics";
import { Cron } from "@/pages/project/cron";
import { Deployments } from "@/pages/project/deployments";
import { Domains } from "@/pages/project/domains";
import { Environment } from "@/pages/project/environment";
import { ProjectLogs } from "@/pages/project/logs";
import { Redirects } from "@/pages/project/redirects";
import { Settings } from "@/pages/project/settings";
import { Speed } from "@/pages/project/speed";
import { Webhooks } from "@/pages/project/webhooks";
import { Rocket } from "lucide-react";
import { useEffect, useState } from "react";
import { useParams, useSearchParams } from "react-router-dom";

const TABS = [
  "deployments",
  "logs",
  "environment",
  "analytics",
  "speed",
  "cron",
  "redirects",
  "domains",
  "webhooks",
  "settings",
] as const;
type Tab = (typeof TABS)[number];

export default function ProjectPage() {
  const { id = "" } = useParams();
  const [params, setParams] = useSearchParams();
  const [project, setProject] = useState<Project | null | undefined>(undefined);
  const tabParam = params.get("tab");
  const tab: Tab = (TABS as readonly string[]).includes(tabParam ?? "")
    ? (tabParam as Tab)
    : "deployments";
  const setTab = (t: Tab) =>
    setParams(t === "deployments" ? {} : { tab: t }, { replace: true });
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    api.project(id).then(setProject).catch(() => setProject(null));
  }, [id]);

  if (project === null) {
    return (
      <div className="page-enter mx-auto max-w-5xl p-4 sm:p-8">
        <EmptyState
          icon={Rocket}
          title="Project not found"
          hint="The project was deleted, or the link is wrong."
        />
      </div>
    );
  }

  function copyUrl() {
    if (!project?.url) return;
    navigator.clipboard
      .writeText(project.url)
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      })
      .catch(() => {});
  }

  return (
    <div className="page-enter mx-auto max-w-5xl p-4 sm:p-8">
      <div className="relative mb-6">
        <div className="relative flex items-center justify-between">
          <div className="flex min-w-0 items-center gap-3">
            {project && (
              <Avatar
                kind="project"
                id={project.id}
                name={project.name}
                hasAvatar={project.has_avatar}
                className="h-11 w-11 text-base"
              />
            )}
            <div className="min-w-0">
              <h1 className="truncate text-2xl font-semibold tracking-tight">
                {project?.name ?? "Project"}
              </h1>
              {project && (
                <div className="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1.5 text-xs text-muted-foreground">
                  <span className="font-mono">
                    {project.repo_provider}:{project.repo_full_name}
                  </span>
                  <span className="font-mono">{project.repo_branch}</span>
                  {project.url && (
                    <span className="inline-flex items-center overflow-hidden rounded-md border border-border">
                      <a
                        href={project.url}
                        target="_blank"
                        className="max-w-40 truncate px-2 py-1 font-mono transition-colors hover:bg-accent hover:text-foreground sm:max-w-none"
                      >
                        {project.url.replace(/^https?:\/\//, "")}
                      </a>
                      <button
                        onClick={copyUrl}
                        className="border-l border-border px-2 py-1 transition-colors hover:bg-accent hover:text-foreground"
                        title="Copy URL"
                        aria-label="Copy URL"
                      >
                        {copied ? "✓" : "⧉"}
                      </button>
                    </span>
                  )}
                </div>
              )}
            </div>
          </div>
        </div>
      </div>
      {/* Rail carries tab links on lg+; strip stays for mobile. */}
      <div className="-mx-4 mb-6 flex gap-4 overflow-x-auto border-b border-border px-4 sm:mx-0 sm:px-0 lg:hidden">
        {TABS.map((t) => (
          <button
            key={t}
            onClick={() => setTab(t)}
            className={`-mb-px shrink-0 border-b-2 pb-2 text-sm capitalize ${
              tab === t
                ? "border-foreground text-foreground"
                : "border-transparent text-muted-foreground hover:text-foreground"
            }`}
          >
            {t}
          </button>
        ))}
      </div>
      {tab === "deployments" && <Deployments id={id} project={project ?? null} />}
      {tab === "logs" && <ProjectLogs id={id} />}
      {tab === "environment" && <Environment id={id} />}
      {tab === "analytics" && <Analytics id={id} />}
      {tab === "speed" && <Speed id={id} />}
      {tab === "cron" && <Cron id={id} />}
      {tab === "redirects" && <Redirects id={id} />}
      {tab === "domains" && <Domains id={id} />}
      {tab === "webhooks" && <Webhooks id={id} />}
      {tab === "settings" && <Settings id={id} project={project ?? null} />}
    </div>
  );
}
