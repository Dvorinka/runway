// Minimal shadcn-style primitives — extended on demand.
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";
import { api } from "@/lib/api";
import { Check, ChevronsUpDown, Search } from "lucide-react";
import { forwardRef, useEffect, useRef, useState } from "react";

const buttonVariants = cva(
  "inline-flex items-center justify-center gap-2 rounded-md text-sm font-medium transition-colors disabled:opacity-50 disabled:pointer-events-none",
  {
    variants: {
      variant: {
        default: "bg-primary text-primary-foreground hover:bg-primary/90",
        outline: "border border-border hover:bg-accent",
        ghost: "hover:bg-accent",
        destructive: "bg-destructive text-white hover:bg-destructive/90",
      },
      size: { default: "h-9 px-4", sm: "h-8 px-3", lg: "h-10 px-6" },
    },
    defaultVariants: { variant: "default", size: "default" },
  },
);

export const Button = forwardRef<
  HTMLButtonElement,
  React.ButtonHTMLAttributes<HTMLButtonElement> & VariantProps<typeof buttonVariants>
>(({ className, variant, size, ...props }, ref) => (
  <button ref={ref} className={cn(buttonVariants({ variant, size }), className)} {...props} />
));
Button.displayName = "Button";

export function Card({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn("rounded-lg border border-border bg-card text-card-foreground", className)}
      {...props}
    />
  );
}

export function Skeleton({ className }: { className?: string }) {
  return (
    <div className={cn("animate-pulse rounded-lg border border-border bg-card/60", className)} />
  );
}

export const Input = forwardRef<HTMLInputElement, React.InputHTMLAttributes<HTMLInputElement>>(
  ({ className, ...props }, ref) => (
    <input
      ref={ref}
      className={cn(
        "flex h-9 w-full rounded-md border border-border bg-transparent px-3 text-sm",
        "placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring",
        className,
      )}
      {...props}
    />
  ),
);
Input.displayName = "Input";

const badgeVariants = cva(
  "inline-flex items-center rounded-md border px-2 py-0.5 text-xs font-medium",
  {
    variants: {
      variant: {
        default: "border-transparent bg-primary text-primary-foreground",
        secondary: "border-transparent bg-muted text-muted-foreground",
        success: "border-transparent bg-emerald-500/15 text-emerald-400",
        warning: "border-transparent bg-amber-500/15 text-amber-400",
        destructive: "border-transparent bg-destructive/15 text-red-400",
        outline: "border-border text-foreground",
      },
    },
    defaultVariants: { variant: "default" },
  },
);

export function Badge({
  className,
  variant,
  ...props
}: React.HTMLAttributes<HTMLSpanElement> & VariantProps<typeof badgeVariants>) {
  return <span className={cn(badgeVariants({ variant }), className)} {...props} />;
}

export function statusVariant(status: string, conclusion?: string | null) {
  if (conclusion === "succeeded") return "success" as const;
  if (conclusion === "failed" || conclusion === "canceled") return "destructive" as const;
  if (status === "deploy" || status === "prepare" || status === "finalize")
    return "warning" as const;
  return "secondary" as const;
}

// Observed container states that mean "not serving". `removed` is left
// out — deliberate teardown keeps the succeeded badge.
const NOT_SERVING = ["crashed", "dead", "paused", "missing", "orphaned", "stopped", "unhealthy"];

/** What the badge should say once the deploy pipeline finished — the
 *  observed container state wins when it's anything but running. */
export function displayStatus(d: {
  status: string;
  conclusion?: string | null;
  computed_status?: string | null;
}): { label: string; variant: "success" | "destructive" | "warning" | "secondary" } {
  const c = d.computed_status;
  if (d.conclusion === "succeeded" && c && NOT_SERVING.includes(c)) {
    return {
      label: c,
      variant: c === "crashed" || c === "dead" ? "destructive" : "warning",
    };
  }
  return {
    label: d.conclusion ?? d.status,
    variant: statusVariant(d.status, d.conclusion),
  };
}

const RUNNING = ["pending", "prepare", "deploy", "finalize", "queued"];

export function isRunning(status: string, conclusion?: string | null) {
  return !conclusion && RUNNING.includes(status);
}

export function StatusDot({
  status,
  conclusion,
  computed,
  className,
}: {
  status: string;
  conclusion?: string | null;
  computed?: string | null;
  className?: string;
}) {
  const unhealthy =
    conclusion === "succeeded" && computed && NOT_SERVING.includes(computed);
  const bad =
    conclusion === "failed" ||
    conclusion === "canceled" ||
    unhealthy === true && (computed === "crashed" || computed === "dead");
  const ok =
    (conclusion === "succeeded" || status === "active" || status === "running") &&
    !unhealthy;
  const color = bad
    ? "bg-red-500"
    : isRunning(status, conclusion) || unhealthy
      ? "bg-amber-400"
      : ok
        ? "bg-emerald-500"
        : "bg-zinc-500";
  return (
    <span className={cn("relative flex h-2 w-2 shrink-0", className)}>
      {isRunning(status, conclusion) && (
        <span
          className={cn("absolute inline-flex h-full w-full animate-ping rounded-full opacity-60", color)}
        />
      )}
      <span className={cn("relative inline-flex h-2 w-2 rounded-full", color)} />
    </span>
  );
}

/** Entity avatar — `/api/avatars/{kind}/{id}` when set, initial else. */
export function Avatar({
  kind,
  id,
  name,
  hasAvatar,
  className,
}: {
  kind: "user" | "team" | "project";
  id: string | number;
  name: string;
  hasAvatar?: boolean;
  className?: string;
}) {
  const cls = cn(
    "flex h-7 w-7 shrink-0 items-center justify-center overflow-hidden rounded-full bg-muted text-xs font-medium uppercase",
    className,
  );
  if (hasAvatar) {
    return (
      <img src={`/api/avatars/${kind}/${id}`} alt={name} className={cn(cls, "object-cover")} />
    );
  }
  return <div className={cls}>{name.charAt(0)}</div>;
}

/** Avatar preview + upload/remove controls. `onChanged` fires after a
 *  successful mutation so the caller can refetch. */
export function AvatarRow({
  kind,
  id,
  name,
  hasAvatar,
  onChanged,
}: {
  kind: "user" | "team" | "project";
  id: string | number;
  name: string;
  hasAvatar?: boolean;
  onChanged?: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const input = useRef<HTMLInputElement>(null);

  async function upload(f: File) {
    setBusy(true);
    setErr("");
    try {
      if (kind === "user") await api.setAvatar(f);
      else await api.setEntityAvatar(kind, id.toString(), f);
      onChanged?.();
    } catch (e) {
      setErr(e instanceof Error ? e.message : "upload failed");
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    setBusy(true);
    setErr("");
    try {
      if (kind === "user") await api.deleteAvatar();
      else await api.deleteEntityAvatar(kind, id.toString());
      onChanged?.();
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mb-4 flex items-center gap-3">
      <Avatar kind={kind} id={id} name={name} hasAvatar={hasAvatar} className="h-10 w-10 text-sm" />
      <input
        ref={input}
        type="file"
        accept="image/png,image/jpeg,image/webp,image/gif"
        className="hidden"
        onChange={(e) => {
          const f = e.target.files?.[0];
          if (f) upload(f);
          e.target.value = "";
        }}
      />
      <Button type="button" variant="outline" size="sm" disabled={busy} onClick={() => input.current?.click()}>
        {hasAvatar ? "Change" : "Upload"} avatar
      </Button>
      {hasAvatar && (
        <Button type="button" variant="ghost" size="sm" disabled={busy} onClick={remove}>
          Remove
        </Button>
      )}
      {err && <span className="text-xs text-destructive">{err}</span>}
    </div>
  );
}

export interface ComboOption {
  value: string;
  label: string;
  hint?: string;
}

// Searchable dropdown — replaces native <select> (whose popup ignores the
// dark theme and can't filter). Controlled via value/onChange with string
// values; map numbers/ids at the call site.
export function ComboBox({
  value,
  onChange,
  options,
  placeholder = "Select…",
  searchPlaceholder = "Search…",
  emptyText = "No matches",
  disabled,
  className,
}: {
  value: string;
  onChange: (value: string) => void;
  options: ComboOption[];
  placeholder?: string;
  searchPlaceholder?: string;
  emptyText?: string;
  disabled?: boolean;
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const rootRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const selected = options.find((o) => o.value === value);
  const q = query.trim().toLowerCase();
  const filtered = q
    ? options.filter((o) => `${o.label} ${o.hint ?? ""}`.toLowerCase().includes(q))
    : options;

  function close() {
    setOpen(false);
    setQuery("");
    setActive(0);
  }
  function pick(v: string) {
    onChange(v);
    close();
  }

  // Close on outside click / Escape.
  useEffect(() => {
    if (!open) return;
    searchRef.current?.focus();
    const onDown = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) close();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open ]);

  function onTriggerKey(e: React.KeyboardEvent) {
    if (e.key === "ArrowDown" || e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      if (!open) setOpen(true);
      else if (filtered[active]) pick(filtered[active].value);
    } else if (e.key === "ArrowUp" && open && filtered.length) {
      e.preventDefault();
      setActive((a) => (a - 1 + filtered.length) % filtered.length);
    }
  }

  return (
    <div ref={rootRef} className={cn("relative", className)}>
      <button
        type="button"
        disabled={disabled}
        onClick={() => (open ? close() : setOpen(true))}
        onKeyDown={onTriggerKey}
        className="flex h-9 w-full items-center justify-between gap-2 rounded-md border border-border bg-transparent px-3 text-sm transition-colors hover:border-muted-foreground/40 focus:outline-none focus:ring-1 focus:ring-ring disabled:opacity-50"
      >
        <span className={cn("truncate", selected ? "text-foreground" : "text-muted-foreground")}>
          {selected ? selected.label : placeholder}
        </span>
        <ChevronsUpDown className="h-4 w-4 shrink-0 text-muted-foreground" />
      </button>
      {open && (
        <div className="absolute z-50 mt-1 max-h-64 w-full overflow-hidden rounded-md border border-border bg-card shadow-xl">
          <div className="flex items-center gap-2 border-b border-border px-3">
            <Search className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
            <input
              ref={searchRef}
              value={query}
              onChange={(e) => {
                setQuery(e.target.value);
                setActive(0);
              }}
              onKeyDown={(e) => {
                if (e.key === "ArrowDown" && filtered.length) {
                  e.preventDefault();
                  setActive((a) => (a + 1) % filtered.length);
                } else if (e.key === "ArrowUp" && filtered.length) {
                  e.preventDefault();
                  setActive((a) => (a - 1 + filtered.length) % filtered.length);
                } else if (e.key === "Enter" && filtered[active]) {
                  e.preventDefault();
                  pick(filtered[active].value);
                }
              }}
              placeholder={searchPlaceholder}
              className="h-9 w-full bg-transparent text-sm text-foreground placeholder:text-muted-foreground focus:outline-none"
            />
          </div>
          <div className="max-h-52 overflow-y-auto p-1">
            {filtered.length === 0 && (
              <p className="px-2 py-3 text-center text-xs text-muted-foreground">{emptyText}</p>
            )}
            {filtered.map((o, i) => {
              const isSel = o.value === value;
              return (
                <button
                  key={o.value}
                  type="button"
                  onMouseEnter={() => setActive(i)}
                  onClick={() => pick(o.value)}
                  className={cn(
                    "flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm transition-colors",
                    i === active ? "bg-accent" : "",
                    isSel ? "text-foreground" : "text-muted-foreground",
                  )}
                >
                  <Check
                    className={cn(
                      "h-4 w-4 shrink-0",
                      isSel ? "text-brand" : "text-transparent",
                    )}
                  />
                  <span className="min-w-0 flex-1 truncate">{o.label}</span>
                  {o.hint && (
                    <span className="shrink-0 truncate text-xs text-muted-foreground/70">
                      {o.hint}
                    </span>
                  )}
                </button>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}
