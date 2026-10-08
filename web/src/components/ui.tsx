// Minimal shadcn-style primitives — extended on demand.
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";
import { forwardRef } from "react";

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

const RUNNING = ["pending", "prepare", "deploy", "finalize", "queued"];

export function isRunning(status: string, conclusion?: string | null) {
  return !conclusion && RUNNING.includes(status);
}

export function StatusDot({
  status,
  conclusion,
  className,
}: {
  status: string;
  conclusion?: string | null;
  className?: string;
}) {
  const bad = conclusion === "failed" || conclusion === "canceled";
  const ok = conclusion === "succeeded" || status === "active" || status === "running";
  const color = bad
    ? "bg-red-500"
    : isRunning(status, conclusion)
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
