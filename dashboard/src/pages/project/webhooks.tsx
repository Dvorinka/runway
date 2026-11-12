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

export function Webhooks({ id }: { id: string }) {
  const [hooks, setHooks] = useState<Webhook[]>([]);
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const [secret, setSecret] = useState("");
  const [error, setError] = useState("");

  const load = () =>
    api.webhooks(id).then((r) => setHooks(r.webhooks)).catch((e) => setError(e.message));
  useEffect(() => {
    load();
  }, [id]);

  async function add(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    try {
      await api.createWebhook(id, { name, url, secret: secret || undefined });
      setName("");
      setUrl("");
      setSecret("");
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed");
    }
  }

  return (
    <>
      <form onSubmit={add} className="mb-4 grid grid-cols-1 sm:grid-cols-[1fr_2fr_1fr_auto] gap-2">
        <Input placeholder="Name" value={name} onChange={(e) => setName(e.target.value)} required />
        <Input
          placeholder="https://example.com/hook"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          required
        />
        <Input
          placeholder="secret (optional)"
          value={secret}
          onChange={(e) => setSecret(e.target.value)}
        />
        <Button type="submit" size="sm">
          Add
        </Button>
      </form>
      <Err msg={error} />
      <div className="stagger grid grid-cols-1 gap-2">
        {hooks.map((w) => (
          <Card key={w.id} className="flex items-center justify-between p-3">
            <div>
              <div className="text-sm font-medium">{w.name}</div>
              <div className="font-mono text-xs text-muted-foreground">
                {w.url} {w.has_secret && "· signed"} {w.events.length > 0 && `· ${w.events.join(",")}`}
              </div>
            </div>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => api.deleteWebhook(id, w.id).then(load).catch(() => {})}
            >
              Delete
            </Button>
          </Card>
        ))}
        {hooks.length === 0 && (
          <div className="rounded-lg border border-dashed border-border p-8 text-center">
            <div className="mx-auto mb-3 flex h-10 w-10 items-center justify-center rounded-full border border-border bg-accent/50">
              <WebhookIcon className="h-5 w-5 text-muted-foreground" />
            </div>
            <p className="text-xs text-muted-foreground">
              No webhooks yet — get signed POSTs for every deployment event.
            </p>
          </div>
        )}
      </div>
    </>
  );
}

