// Repo docs rendered on GitHub — target for settings-card "Learn more" links.
export const DOCS = "https://github.com/Dvorinka/runway/blob/main/docs";

export const inputCls =
  "w-full rounded-md border border-border bg-transparent px-3 py-2 font-mono text-xs focus:outline-none focus:ring-1 focus:ring-ring";

export function Err({ msg }: { msg: string }) {
  return msg ? <p className="mt-2 text-xs text-destructive">{msg}</p> : null;
}
