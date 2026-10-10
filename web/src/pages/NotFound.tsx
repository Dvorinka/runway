import { Button } from "@/components/ui";
import { Link } from "react-router-dom";

export default function NotFound() {
  return (
    <div className="page-enter mx-auto flex max-w-5xl flex-col items-center pt-32 text-center">
      <p className="font-mono text-sm text-brand">404</p>
      <h1 className="mt-2 text-xl font-semibold">Nothing on this runway</h1>
      <p className="mt-2 text-sm text-muted-foreground">
        The page you requested doesn't exist or was moved.
      </p>
      <Link to="/projects" className="mt-6">
        <Button size="sm" variant="outline">
          Back to projects
        </Button>
      </Link>
    </div>
  );
}
