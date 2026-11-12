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

export function Domains({ id }: { id: string }) {
  const [domains, setDomains] = useState<Domain[]>([]);
  const [auto, setAuto] = useState<{ label: string; host: string }[]>([]);
  const [hostname, setHostname] = useState("");
  const [error, setError] = useState("");
  const [copied, setCopied] = useState("");

  const load = () => {
    api.domains(id).then((r) => setDomains(r.domains)).catch((e) => setError(e.message));
    api
      .deployments(id)
      .then((r) => {
        const d = r.deployments[0];
        const seen = new Set<string>();
        const rows: { label: string; host: string }[] = [];
        const push = (label: string, host?: string | null) => {
          if (host && !seen.has(host)) {
            seen.add(host);
            rows.push({ label, host });
          }
        };
        push("Environment", d?.urls?.environment);
        push("Branch preview", d?.urls?.branch);
        push("Immutable", d?.urls?.immutable);
        setAuto(rows);
      })
      .catch(() => {});
  };
  useEffect(() => {
    load();
  }, [id]);

  function copy(host: string) {
    navigator.clipboard
      .writeText(`https://${host}`)
      .then(() => {
        setCopied(host);
        setTimeout(() => setCopied(""), 1500);
      })
      .catch(() => {});
  }

  async function add(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    try {
      await api.addDomain(id, hostname);
      setHostname("");
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed");
    }
  }

  return (
    <>
      {auto.length > 0 && (
        <>
          <SectionHead icon={Globe} title="Automatic">
            Issued for every deployment — environment, branch previews, and the immutable build URL.
          </SectionHead>
          <div className="stagger mb-6 grid grid-cols-1 gap-2">
            {auto.map((a) => (
              <Card key={a.host} className="flex items-center justify-between p-3">
                <div className="flex min-w-0 items-center gap-2">
                  <Badge variant="secondary">{a.label}</Badge>
                  <a
                    href={`https://${a.host}`}
                    target="_blank"
                    rel="noreferrer"
                    className="min-w-0 flex-1 truncate font-mono text-sm hover:text-foreground"
                  >
                    {a.host}
                  </a>
                </div>
                <Button variant="ghost" size="sm" onClick={() => copy(a.host)} title="Copy URL" aria-label="Copy URL">
                  {copied === a.host ? "✓" : "⧉"}
                </Button>
              </Card>
            ))}
          </div>
        </>
      )}
      <SectionHead icon={Link2} title="Custom domains">
        Point DNS at this instance (or assign via Cloudflare), then verify.
      </SectionHead>
      <form onSubmit={add} className="mb-4 flex gap-2">
        <Input
          placeholder="app.example.com"
          value={hostname}
          onChange={(e) => setHostname(e.target.value)}
          required
        />
        <Button type="submit" size="sm" className="shrink-0">
          Add domain
        </Button>
      </form>
      <Err msg={error} />
      <div className="stagger grid grid-cols-1 gap-2">
        {domains.map((d) => (
          <Card key={d.id} className="flex items-center justify-between p-3">
            <div className="font-mono text-sm">{d.hostname}</div>
            <div className="flex items-center gap-2">
              <Badge variant={d.status === "active" ? "success" : "warning"}>{d.status}</Badge>
              {d.status !== "active" && (
                <>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() =>
                      api.assignCloudflareDomain(id, d.id).then(load).catch((e) => setError(e.message))
                    }
                  >
                    Assign via Cloudflare
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() =>
                      api.verifyDomain(id, d.id).then(load).catch((e) => setError(e.message))
                    }
                  >
                    Verify
                  </Button>
                </>
              )}
              <Button
                variant="ghost"
                size="sm"
                onClick={() => api.deleteDomain(id, d.id).then(load).catch(() => {})}
              >
                Remove
              </Button>
            </div>
          </Card>
        ))}
        {domains.length === 0 && (
          <EmptyState
            icon={Link2}
            title="Add a domain"
            hint="Add a domain you own, point its DNS at this instance, then verify."
          />
        )}
      </div>
    </>
  );
}

