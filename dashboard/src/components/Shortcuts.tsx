import { useEffect, useState } from "react";

export const GO_TO: { key: string; label: string; to: string }[] = [
  { key: "o", label: "Overview", to: "/" },
  { key: "p", label: "Projects", to: "/projects" },
  { key: "d", label: "Deployments", to: "/deployments" },
  { key: "a", label: "Activity", to: "/activity" },
  { key: "n", label: "Notifications", to: "/notifications" },
  { key: "t", label: "Teams", to: "/teams" },
  { key: "s", label: "Settings", to: "/settings" },
];

function typingTarget(el: EventTarget | null): boolean {
  const node = el as HTMLElement | null;
  return !!node && (node.tagName === "INPUT" || node.tagName === "TEXTAREA" || node.isContentEditable);
}

export function Shortcuts() {
  const [open, setOpen] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpen(false);
        return;
      }
      if (typingTarget(e.target) || e.metaKey || e.ctrlKey || e.altKey) return;
      if (e.key !== "?") return;
      if (!open && document.querySelector("[role=dialog]")) return;
      e.preventDefault();
      setOpen((v) => !v);
    };
    const onOpen = () => setOpen(true);
    window.addEventListener("keydown", onKey);
    window.addEventListener("runway:shortcuts", onOpen);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("runway:shortcuts", onOpen);
    };
  }, [open]);

  if (!open) return null;
  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/60 pt-[18vh] backdrop-blur-sm"
      onMouseDown={() => setOpen(false)}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Keyboard shortcuts"
        className="w-full max-w-sm overflow-hidden rounded-lg border border-border bg-card shadow-2xl"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="border-b border-border px-4 py-3 text-sm font-medium">Shortcuts</div>
        <ul className="px-4 py-2 text-sm">
          <li className="flex items-center justify-between py-1.5">
            <span>Find</span>
            <span className="flex gap-1">
              <kbd className="rounded border border-border bg-muted/50 px-1 font-mono text-[10px]">⌘K</kbd>
              <kbd className="rounded border border-border bg-muted/50 px-1 font-mono text-[10px]">F</kbd>
            </span>
          </li>
          {GO_TO.map((g) => (
            <li key={g.key} className="flex items-center justify-between py-1.5">
              <span>{g.label}</span>
              <span className="flex gap-1">
                <kbd className="rounded border border-border bg-muted/50 px-1 font-mono text-[10px]">G</kbd>
                <kbd className="rounded border border-border bg-muted/50 px-1 font-mono text-[10px]">
                  {g.key.toUpperCase()}
                </kbd>
              </span>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
