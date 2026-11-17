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

export function ProjectLogs({ id }: { id: string }) {
  const [lines, setLines] = useState<string[]>([]);
  const [filter, setFilter] = useState("");
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const pull = () => api.projectLogs(id).then((t) => setLines(t.split("\n").filter(Boolean))).catch(() => {});
    pull();
    const t = setInterval(pull, 5000);
    return () => clearInterval(t);
  }, [id]);

  useEffect(() => {
    ref.current?.scrollTo(0, ref.current.scrollHeight);
  }, [lines.length]);

  const shown = filter
    ? lines.filter((l) => l.toLowerCase().includes(filter.toLowerCase()))
    : lines;

  return (
    <div className="overflow-hidden rounded-lg border border-border bg-black/40">
      <div className="flex items-center justify-between border-b border-border px-3 py-2">
        <span className="flex items-center gap-2 text-xs text-muted-foreground">
          <span className="relative flex h-1.5 w-1.5">
            <span className="absolute h-full w-full animate-ping rounded-full bg-emerald-500 opacity-60" />
            <span className="relative h-1.5 w-1.5 rounded-full bg-emerald-500" />
          </span>
          live · merged build + runtime log across recent deployments
        </span>
        <input
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="filter…"
          className="w-40 bg-transparent text-xs text-muted-foreground placeholder:text-muted-foreground/50 focus:text-foreground focus:outline-none"
        />
      </div>
      <div ref={ref} className="max-h-[60vh] overflow-auto p-4 font-mono text-xs leading-relaxed text-zinc-300">
        {shown.length === 0 ? (
          <span className="text-muted-foreground">no logs yet</span>
        ) : (
          shown.map((l, i) => {
            const [ts, dep, ...rest] = l.split("\t");
            return (
              <div key={i} className="flex gap-3">
                <span className="shrink-0 text-muted-foreground/50">
                  {ts?.slice(11, 19)}
                </span>
                <span className="shrink-0 text-muted-foreground">{dep}</span>
                <span className="whitespace-pre-wrap break-all">{rest.join("\t")}</span>
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
