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

export function Environment({ id }: { id: string }) {
  const [vars, setVars] = useState<EnvVar[]>([]);
  const [key, setKey] = useState("");
  const [value, setValue] = useState("");
  const [scope, setScope] = useState("");
  const [scopes, setScopes] = useState<string[]>([]);
  const [bulk, setBulk] = useState(false);
  const [bulkText, setBulkText] = useState("");
  const [error, setError] = useState("");

  const load = () => {
    api.getEnv(id).then((r) => setVars(r.env)).catch((e) => setError(e.message));
    api
      .project(id)
      .then((p) =>
        setScopes(
          ((p.environments ?? []) as { id?: string }[])
            .map((e) => e.id ?? "")
            .filter(Boolean),
        ),
      )
      .catch(() => {});
  };
  useEffect(() => {
    load();
  }, [id]);

  async function add(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    try {
      await api.patchEnv(id, [{ key, value, environment: scope || undefined }]);
      setKey("");
      setValue("");
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed");
    }
  }

  async function addBulk(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    const pairs = bulkText
      .split("\n")
      .map((l) => l.trim())
      .filter((l) => l && !l.startsWith("#") && l.includes("="))
      .map((l) => {
        const i = l.indexOf("=");
        return { key: l.slice(0, i).trim(), value: l.slice(i + 1).trim() };
      })
      .filter((p) => p.key);
    if (pairs.length === 0) {
      setError("no KEY=value lines found");
      return;
    }
    try {
      await api.patchEnv(id, pairs);
      setBulkText("");
      setBulk(false);
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed");
    }
  }

  return (
    <>
      {bulk ? (
        <form onSubmit={addBulk} className="mb-4 grid grid-cols-1 gap-2">
          <textarea
            value={bulkText}
            onChange={(e) => setBulkText(e.target.value)}
            placeholder={"KEY=value\nANOTHER=thing\n# comments ignored"}
            rows={6}
            autoFocus
            className="w-full rounded-md border border-border bg-transparent px-3 py-2 font-mono text-sm placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring"
          />
          <div className="flex gap-2">
            <Button type="submit" size="sm" disabled={!bulkText.trim()}>
              Add all
            </Button>
            <Button type="button" variant="ghost" size="sm" onClick={() => setBulk(false)}>
              Cancel
            </Button>
          </div>
        </form>
      ) : (
        <form onSubmit={add} className="mb-4 grid grid-cols-1 sm:grid-cols-[1fr_1fr_130px_auto_auto] gap-2">
          <Input placeholder="KEY" value={key} onChange={(e) => setKey(e.target.value)} required />
          <Input placeholder="value" value={value} onChange={(e) => setValue(e.target.value)} />
          <ComboBox
            value={scope}
            onChange={setScope}
            options={[
              { value: "", label: "All envs" },
              ...scopes.map((s) => ({ value: s, label: s })),
            ]}
            placeholder="scope…"
          />
          <Button type="submit" size="sm">
            Add
          </Button>
          <Button type="button" variant="outline" size="sm" onClick={() => setBulk(true)}>
            Paste .env
          </Button>
        </form>
      )}
      <Err msg={error} />
      <div className="stagger grid grid-cols-1 gap-2">
        {vars.map((v) => (
          <Card key={`${v.key}:${v.environment}`} className="flex items-center justify-between p-3">
            <div className="font-mono text-sm">
              {v.key}
              {v.environment && (
                <span className="ml-2 text-xs text-muted-foreground">({v.environment})</span>
              )}
            </div>
            <div className="flex items-center gap-3">
              <span className="font-mono text-xs text-muted-foreground">{v.value || "••••"}</span>
              <Button
                variant="ghost"
                size="sm"
                onClick={() =>
                  api
                    .patchEnv(id, [
                      { key: v.key, environment: v.environment ?? undefined, delete: true },
                    ])
                    .then(load)
                    .catch(() => {})
                }
              >
                Remove
              </Button>
            </div>
          </Card>
        ))}
        {vars.length === 0 && (
          <EmptyState
            icon={KeyRound}
            title="No environment variables yet"
            hint="Add a key above, or paste a .env file."
          />
        )}
      </div>
    </>
  );
}

