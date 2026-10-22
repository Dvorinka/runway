import { Button } from "@/components/ui";
import { Link } from "react-router-dom";

export default function NotFound() {
  return (
    <div className="page-enter relative mx-auto flex min-h-[70vh] max-w-5xl flex-col items-center justify-center overflow-hidden text-center">
      <div className="pointer-events-none absolute inset-0" aria-hidden>
        <div className="absolute left-1/2 top-1/2 h-16 w-[160%] -translate-x-1/2 -translate-y-1/2 -rotate-[5deg] bg-accent/20">
          <div className="absolute inset-x-0 top-1/2 h-px -translate-y-1/2 bg-[repeating-linear-gradient(90deg,var(--color-foreground)_0_28px,transparent_28px_56px)] opacity-20" />
        </div>
      </div>
      <p className="relative font-mono text-sm text-muted-foreground">404</p>
      <h1 className="relative mt-2 text-xl font-semibold">Nothing on this runway</h1>
      <p className="relative mt-2 text-sm text-muted-foreground">
        The page you requested doesn't exist or was moved.
      </p>
      <Link to="/projects" className="relative mt-6">
        <Button size="sm" variant="outline">
          Back to projects
        </Button>
      </Link>
    </div>
  );
}
