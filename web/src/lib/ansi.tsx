import type { ReactNode } from "react";

const COLORS: Record<number, string> = {
  30: "text-zinc-600",
  31: "text-red-400",
  32: "text-emerald-400",
  33: "text-amber-400",
  34: "text-blue-400",
  35: "text-purple-400",
  36: "text-cyan-400",
  37: "text-zinc-300",
  90: "text-zinc-500",
  91: "text-red-300",
  92: "text-emerald-300",
  93: "text-amber-300",
  94: "text-blue-300",
  95: "text-purple-300",
  96: "text-cyan-300",
  97: "text-zinc-100",
};

const SGR = /\x1b\[([0-9;]*)m/g;
const CSI = /\x1b\[[0-9;?]*([A-Za-z])/g;

export function Ansi({ text }: { text: string }) {
  let t = text.includes("\r") ? text.slice(text.lastIndexOf("\r") + 1) : text;
  t = t.replace(CSI, (s, c) => (c === "m" ? s : ""));

  const parts: ReactNode[] = [];
  let cls = "";
  let bold = false;
  let last = 0;
  let key = 0;
  let m: RegExpExecArray | null;
  while ((m = SGR.exec(t))) {
    if (m.index > last) {
      parts.push(
        <span key={key++} className={cls}>
          {t.slice(last, m.index)}
        </span>,
      );
    }
    for (const code of m[1].split(";").map(Number)) {
      if (code === 0) {
        cls = "";
        bold = false;
      } else if (code === 1) bold = true;
      else if (code === 22) bold = false;
      else if (code === 39) cls = "";
      else if (COLORS[code]) cls = COLORS[code];
    }
    last = SGR.lastIndex;
  }
  if (last < t.length) {
    parts.push(
      <span key={key++} className={cls + (bold ? " font-semibold" : "")}>
        {t.slice(last)}
      </span>,
    );
  }
  return <>{parts}</>;
}
