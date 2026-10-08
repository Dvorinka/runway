import { useEffect, useState } from "react";
import { useParams } from "react-router-dom";
import { api } from "@/lib/api";

type EnvStatus = {
  name: string;
  slug: string;
  status: "operational" | "degraded" | "outage" | "unknown";
  uptime_30d: number;
  since: string;
};

type StatusData = {
  name: string;
  description?: string;
  status: string;
  environments: EnvStatus[];
  updated_at: string;
};

const BANNER: Record<string, { text: string; cls: string; dot: string }> = {
  operational: { text: "All systems operational", cls: "text-emerald-400", dot: "bg-emerald-500" },
  degraded: { text: "Partially degraded", cls: "text-amber-400", dot: "bg-amber-500" },
  outage: { text: "Service disruption", cls: "text-red-400", dot: "bg-red-500" },
  unknown: { text: "Status unknown", cls: "text-muted-foreground", dot: "bg-muted-foreground" },
};

const DOT: Record<string, string> = {
  operational: "bg-emerald-500",
  degraded: "bg-amber-500",
  outage: "bg-red-500",
  unknown: "bg-muted-foreground",
};

export default function StatusPage() {
  const { slug } = useParams<{ slug: string }>();
  const [data, setData] = useState<StatusData | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!slug) return;
    const pull = () =>
      api
        .statusPage(slug)
        .then((d) => setData(d as StatusData))
        .catch((e) => setError(e instanceof Error ? e.message : "unavailable"));
    pull();
    const t = setInterval(pull, 30_000);
    return () => clearInterval(t);
  }, [slug]);

  const banner = BANNER[data?.status ?? "unknown"] ?? BANNER.unknown;

  return (
    <div className="flex min-h-screen items-start justify-center bg-background px-4 py-24 text-foreground">
      <div className="w-full max-w-xl">
        {error && <p className="text-center text-sm text-muted-foreground">{error}</p>}
        {!data && !error && <p className="text-center text-sm text-muted-foreground">Loading…</p>}
        {data && (
          <>
            <p className="mb-1 text-center text-xs uppercase tracking-widest text-muted-foreground">
              {data.name}
            </p>
            <h1 className={`mb-1 text-center text-2xl font-semibold ${banner.cls}`}>
              {banner.text}
            </h1>
            <p className="mb-10 text-center text-xs text-muted-foreground">
              Updated {new Date(data.updated_at).toLocaleTimeString()}
            </p>
            <div className="divide-y divide-border rounded-lg border border-border">
              {data.environments.length === 0 && (
                <p className="p-4 text-center text-sm text-muted-foreground">
                  No environments reporting yet.
                </p>
              )}
              {data.environments.map((e) => (
                <div key={e.slug} className="flex items-center justify-between px-4 py-3">
                  <div className="flex items-center gap-2.5">
                    <span className={`h-2 w-2 rounded-full ${DOT[e.status] ?? DOT.unknown}`} />
                    <span className="text-sm">{e.name}</span>
                  </div>
                  <div className="text-right">
                    <div className="text-sm capitalize">{e.status}</div>
                    <div className="text-xs text-muted-foreground">
                      {e.uptime_30d.toFixed(1)}% · 30d
                    </div>
                  </div>
                </div>
              ))}
            </div>
            <p className="mt-10 text-center text-xs text-muted-foreground">
              Powered by <span className="font-medium">Runway</span>
            </p>
          </>
        )}
      </div>
    </div>
  );
}
