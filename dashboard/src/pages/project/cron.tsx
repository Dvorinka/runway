import {
  api,
  projectEventsStream,
  type CronJob,
  type Deployment,
  type Domain,
  type EnvVar,
  type Project,
  type RedirectRule,
  type RemoteNode,
  type Webhook,
} from "@/lib/api";
import { Avatar, AvatarRow, Badge, Button, Card, CardFooter, ComboBox, EmptyState, Input, SectionHead, Skeleton, StatusDot, displayStatus } from "@/components/ui";
import { DeploymentRow } from "@/components/DeploymentRow";
import { filesToTarGz } from "@/lib/tarball";
import { cn, timeAgo } from "@/lib/utils";
import { Globe, Link2, Rocket, Webhook as WebhookIcon, ArrowRightLeft, ListFilter, CheckCircle2, Circle, KeyRound, Clock } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { DOCS, Err, inputCls } from "./shared";

export function Cron({ id }: { id: string }) {
  const [jobs, setJobs] = useState<CronJob[]>([]);
  const [name, setName] = useState("");
  const [schedule, setSchedule] = useState("");
  const [branch, setBranch] = useState("main");
  const [error, setError] = useState("");

  const load = () => api.cron(id).then((r) => setJobs(r.cron_jobs)).catch((e) => setError(e.message));
  useEffect(() => {
    load();
  }, [id]);

  async function add(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    try {
      await api.createCron(id, { name, schedule, branch });
      setName("");
      setSchedule("");
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed");
    }
  }

  return (
    <>
      <form onSubmit={add} className="mb-4 grid grid-cols-1 sm:grid-cols-[1fr_1fr_120px_auto] gap-2">
        <Input placeholder="Name" value={name} onChange={(e) => setName(e.target.value)} required />
        <Input
          placeholder="every 30 minutes"
          value={schedule}
          onChange={(e) => setSchedule(e.target.value)}
          required
          title="every N minutes, every N hours, */N * * * *, or bare minutes"
        />
        <Input placeholder="branch" value={branch} onChange={(e) => setBranch(e.target.value)} />
        <Button type="submit" size="sm">
          Add
        </Button>
      </form>
      <div className="mb-4 flex flex-wrap gap-1.5">
        {[
          ["15 min", "every 15 minutes"],
          ["hourly", "every 1 hours"],
          ["every 6h", "every 6 hours"],
          ["daily", "every 24 hours"],
        ].map(([label, s]) => (
          <button
            key={s}
            type="button"
            onClick={() => setSchedule(s)}
            className={`rounded-full border px-2.5 py-0.5 text-xs transition-colors ${
              schedule === s
                ? "border-foreground/25 bg-accent text-foreground"
                : "border-border text-muted-foreground hover:text-foreground"
            }`}
          >
            {label}
          </button>
        ))}
      </div>
      <Err msg={error} />
      <div className="stagger grid grid-cols-1 gap-2">
        {jobs.map((j) => (
          <Card key={j.id} className="flex items-center justify-between p-3">
            <div>
              <div className="text-sm font-medium">
                {j.name} <span className="font-mono text-xs text-muted-foreground">{j.schedule}</span>
              </div>
              <div className="text-xs text-muted-foreground">
                {j.branch} · next {j.next_run_at ? new Date(j.next_run_at).toLocaleString() : "—"}
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Badge variant={j.enabled ? "success" : "secondary"}>
                {j.enabled ? "enabled" : "disabled"}
              </Badge>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => api.patchCron(id, j.id, !j.enabled).then(load).catch(() => {})}
              >
                {j.enabled ? "Disable" : "Enable"}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => api.deleteCron(id, j.id).then(load).catch(() => {})}
              >
                Delete
              </Button>
            </div>
          </Card>
        ))}
        {jobs.length === 0 && (
          <EmptyState
            icon={Clock}
            title="No cron jobs yet"
            hint="Schedule deployments of a branch on an interval."
          />
        )}
      </div>
    </>
  );
}

