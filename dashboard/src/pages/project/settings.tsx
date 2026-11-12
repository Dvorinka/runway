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

import {
  DockerBuild,
  DeployRules,
  Environments,
  Firewall,
  HealthCheck,
  PreviewDomain,
  Retention,
  StatusPageCard,
} from "./settings-ops";

export function Settings({ id, project }: { id: string; project: Project | null }) {
  const [nodes, setNodes] = useState<RemoteNode[]>([]);
  const [error, setError] = useState("");
  const [msg, setMsg] = useState("");
  const importRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    api.nodes().then((r) => setNodes(r.nodes)).catch(() => {});
  }, []);

  async function assign(nodeId: string) {
    setError("");
    setMsg("");
    try {
      await api.patchProject(id, { remote_node_id: nodeId || null });
      setMsg("Saved.");
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed");
    }
  }

  async function doExport() {
    const data = await api.exportProject(id);
    const blob = new Blob([JSON.stringify(data, null, 2)], { type: "application/json" });
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = `${project?.name ?? "project"}-export.json`;
    a.click();
    URL.revokeObjectURL(a.href);
  }

  async function doImport(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    if (!file) return;
    setError("");
    setMsg("");
    try {
      await api.importProject(id, JSON.parse(await file.text()));
      setMsg("Imported. Config merged; env vars and redirect rules added.");
    } catch (err) {
      setError(err instanceof Error ? err.message : "import failed");
    }
    e.target.value = "";
  }

  const [avatarKey, setAvatarKey] = useState(0);
  return (
    <div className="grid grid-cols-1 gap-4">
      {project && (
        <Card className="p-4">
          <h3 className="mb-2 text-sm font-medium">Avatar</h3>
          <AvatarRow
            key={avatarKey}
            kind="project"
            id={project.id}
            name={project.name}
            hasAvatar={project.has_avatar}
            onChanged={() => {
              project.has_avatar = !project.has_avatar;
              setAvatarKey((k) => k + 1);
            }}
          />
        </Card>
      )}
      <Card className="p-4">
        <h3 className="mb-2 text-sm font-medium">Remote Docker node</h3>
        <p className="mb-3 text-xs text-muted-foreground">
          Deploy to a remote Docker daemon. Requires the node published port to be reachable from
          Traefik.
        </p>
        <ComboBox
          value={project?.remote_node_id ?? ""}
          onChange={(v) => assign(v)}
          options={[
            { value: "", label: "Local daemon" },
            ...nodes.map((n) => ({
              value: n.id,
              label: n.name,
              hint: `${n.host}${n.status !== "online" ? " — offline" : ""}`,
            })),
          ]}
          placeholder="Local daemon"
          searchPlaceholder="Search nodes…"
        />
      </Card>
      <Environments id={id} project={project} />
      <DockerBuild id={id} project={project} />
      <HealthCheck id={id} project={project} />
      <Retention id={id} project={project} />
      <DeployRules id={id} project={project} />
      <StatusPageCard id={id} project={project} />
      <Firewall id={id} project={project} />
      <PreviewDomain id={id} project={project} />
      <Card className="p-4">
        <h3 className="mb-2 text-sm font-medium">Export / import</h3>
        <div className="flex gap-2">
          <Button variant="outline" size="sm" onClick={() => doExport().catch(() => {})}>
            Export JSON
          </Button>
          <Button variant="outline" size="sm" onClick={() => importRef.current?.click()}>
            Import JSON
          </Button>
          <input ref={importRef} type="file" accept=".json" hidden onChange={doImport} />
        </div>
      </Card>
      {project && <DangerZone project={project} />}
      <Err msg={error} />
      {msg && <p className="text-xs text-muted-foreground">{msg}</p>}
    </div>
  );
}

export function DangerZone({ project }: { project: Project }) {
  const nav = useNavigate();
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");

  async function destroy() {
    setBusy(true);
    setErr("");
    try {
      await api.deleteProject(project.id, confirm);
      nav("/projects");
    } catch (e) {
      setErr(e instanceof Error ? e.message : "failed");
      setBusy(false);
    }
  }

  return (
    <Card className="border-destructive/40 p-4">
      <h3 className="mb-1 text-sm font-medium text-destructive">Delete project</h3>
      <p className="mb-3 text-xs text-muted-foreground">
        Removes all deployments, containers, domains and edge config. Type{" "}
        <span className="font-mono text-foreground">{project.name}</span> to confirm.
      </p>
      <div className="flex gap-2">
        <Input
          value={confirm}
          onChange={(e) => setConfirm(e.target.value)}
          placeholder={project.name}
          className="max-w-56"
        />
        <Button
          variant="destructive"
          size="sm"
          disabled={busy || confirm !== project.name}
          onClick={destroy}
        >
          Delete
        </Button>
      </div>
      {err && <p className="mt-2 text-xs text-destructive">{err}</p>}
    </Card>
  );
}

