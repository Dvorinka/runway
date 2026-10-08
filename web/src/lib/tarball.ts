// Minimal tar.gz builder for drag-drop deploys: ustar headers + gzip via
// CompressionStream. No dependency; supports the small file sets a user
// would drop (static sites), not a general-purpose archiver.

function tarHeader(name: string, size: number): Uint8Array {
  const buf = new Uint8Array(512);
  const enc = new TextEncoder();
  const put = (str: string, off: number, len: number) =>
    buf.set(enc.encode(str).slice(0, len), off);

  // Field sizes per POSIX ustar.
  put(name.length > 99 ? name.slice(-99) : name, 0, 100);
  put("0000644\0", 100, 8); // mode
  put("0000000\0", 108, 8); // uid
  put("0000000\0", 116, 8); // gid
  put(size.toString(8).padStart(11, "0") + "\0", 124, 12);
  put(Math.floor(Date.now() / 1000).toString(8).padStart(11, "0") + "\0", 136, 12);
  buf.set(enc.encode("ustar\0"), 257);
  buf.set(enc.encode("00"), 263);
  // Header checksum: bytes 148-156, computed with field treated as spaces.
  buf.fill(0x20, 148, 156);
  let sum = 0;
  for (const b of buf) sum += b;
  put(sum.toString(8).padStart(6, "0") + "\0 ", 148, 8);
  return buf;
}

async function gzip(data: Uint8Array): Promise<Blob> {
  const cs = new CompressionStream("gzip");
  const stream = new Blob([data.buffer as ArrayBuffer]).stream().pipeThrough(cs);
  return new Response(stream).blob();
}

/** Pack dropped files into a gzipped tar. A lone .html file becomes index.html
 * so it serves at the site root. Paths are sanitized to a flat prefix. */
export async function filesToTarGz(files: File[]): Promise<Blob> {
  const entries: { name: string; data: Uint8Array }[] = [];
  for (const [i, f] of files.entries()) {
    const raw = (f as File & { webkitRelativePath?: string }).webkitRelativePath || f.name;
    let name = raw.replace(/^\/+/, "").replace(/\.\./g, "");
    if (!name) name = `file-${i}`;
    if (files.length === 1 && name.toLowerCase().endsWith(".html") && name !== "index.html")
      name = "index.html";
    entries.push({ name, data: new Uint8Array(await f.arrayBuffer()) });
  }
  const chunks: Uint8Array[] = [];
  for (const e of entries) {
    chunks.push(tarHeader(e.name, e.data.length));
    chunks.push(e.data);
    const pad = (512 - (e.data.length % 512)) % 512;
    if (pad) chunks.push(new Uint8Array(pad));
  }
  chunks.push(new Uint8Array(1024)); // end-of-archive marker
  const size = chunks.reduce((n, c) => n + c.length, 0);
  const tar = new Uint8Array(size);
  let off = 0;
  for (const c of chunks) {
    tar.set(c, off);
    off += c.length;
  }
  return gzip(tar);
}
