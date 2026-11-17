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

export function Redirects({ id }: { id: string }) {
  const [rules, setRules] = useState<RedirectRule[]>([]);
  const [source, setSource] = useState("");
  const [target, setTarget] = useState("");
  const [code, setCode] = useState("301");
  const [error, setError] = useState("");

  const load = () =>
    api.redirects(id).then((r) => setRules(r.redirect_rules)).catch((e) => setError(e.message));
  useEffect(() => {
    load();
  }, [id]);

  async function add(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    try {
      await api.createRedirect(id, {
        source_path: source,
        target_url: target,
        status_code: Number(code),
      });
      setSource("");
      setTarget("");
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed");
    }
  }

  return (
    <>
      <form onSubmit={add} className="mb-4 grid grid-cols-1 sm:grid-cols-[1fr_1fr_90px_auto] gap-2">
        <Input
          placeholder="/old-path"
          value={source}
          onChange={(e) => setSource(e.target.value)}
          required
        />
        <Input
          placeholder="https://example.com/new"
          value={target}
          onChange={(e) => setTarget(e.target.value)}
          required
        />
        <ComboBox
          value={code}
          onChange={setCode}
          options={["301", "302", "307", "308"].map((c) => ({ value: c, label: c }))}
          placeholder="Code…"
        />
        <Button type="submit" size="sm">
          Add
        </Button>
      </form>
      <Err msg={error} />
      <div className="stagger grid grid-cols-1 gap-2">
        {rules.map((r) => (
          <Card key={r.id} className="flex items-center justify-between p-3">
            <div className="font-mono text-sm">
              {r.source_path} → {r.target_url}
              <span className="ml-2 text-xs text-muted-foreground">{r.status_code}</span>
            </div>
            <div className="flex items-center gap-2">
              <Badge variant={r.enabled ? "success" : "secondary"}>
                {r.enabled ? "on" : "off"}
              </Badge>
              <Button
                variant="ghost"
                size="sm"
                onClick={() =>
                  api.patchRedirect(id, r.id, { enabled: !r.enabled }).then(load).catch(() => {})
                }
              >
                {r.enabled ? "Disable" : "Enable"}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => api.deleteRedirect(id, r.id).then(load).catch(() => {})}
              >
                Delete
              </Button>
            </div>
          </Card>
        ))}
        {rules.length === 0 && (
          <div className="rounded-lg border border-dashed border-border p-8 text-center">
            <div className="mx-auto mb-3 flex h-10 w-10 items-center justify-center rounded-full border border-border bg-accent/50">
              <ArrowRightLeft className="h-5 w-5 text-muted-foreground" />
            </div>
            <p className="text-xs text-muted-foreground">No redirect rules yet — /old-path → new home.</p>
          </div>
        )}
      </div>
    </>
  );
}

