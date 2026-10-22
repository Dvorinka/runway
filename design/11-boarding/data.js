/* RUNWAY — BOARDING · shared data + render helpers */

const STAGES = ["QUEUE","TAXI","CLIMB","FINAL","LANDED"];
const stageIdx = { held:0, taxi:1, climb:2, final:3, landed:4, diverted:4 };

const FLIGHTS = [
  {id:"RWY-9C1E", proj:"api",       preset:"dockerfile", env:"production", branch:"main",       sha:"9c1e44d", commit:"fix: token refresh race",      st:"final",    dur:"48s",   alias:"api-devs-team.deploy.localhost",            health:"probe 200 OK · 214ms",  when:"19:53", note:"alias swap -env-prod pending"},
  {id:"RWY-77BD", proj:"docs",      preset:"vite",       env:"preview",    branch:"docs",       sha:"77bd08a", commit:"docs: self-hosting guide",     st:"climb",    dur:"31s",   alias:"docs-branch-docs.deploy.localhost",        health:"build 41%",             when:"19:54", note:"basicAuth on preview router"},
  {id:"RWY-4F2A", proj:"storefront",preset:"nextjs",     env:"production", branch:"main",       sha:"4f2a9e1", commit:"feat: cart totals rework",     st:"landed",   dur:"2m04s", alias:"storefront-devs-team.deploy.localhost",    health:"healthy · p95 214ms",   when:"19:42", note:"rollback base — keep"},
  {id:"RWY-B3F9", proj:"landing",   preset:"astro",      env:"production", branch:"main",       sha:"b3f97c2", commit:"chore: bump hero copy",        st:"landed",   dur:"38s",   alias:"landing-devs-team.deploy.localhost",       health:"static publish",        when:"18:11", note:"served from data/static"},
  {id:"RWY-2E8C", proj:"storefront",preset:"nextjs",     env:"staging",    branch:"staging",    sha:"2e8c5b0", commit:"wip: checkout experiments",   st:"held",     dur:"—",     alias:"storefront-env-staging.deploy.localhost",  health:"queue depth 1",         when:"19:46", note:"waits on build slot"},
  {id:"RWY-A110", proj:"api",       preset:"dockerfile", env:"preview",    branch:"retry-sched",sha:"a1104f9", commit:"feat: retry scheduler",       st:"diverted", dur:"1m31s", alias:"api-id-a1104f.deploy.localhost",           health:"runner OOM killed",     when:"19:39", note:"container stopped, strip kept"},
  {id:"RWY-8C31", proj:"api",       preset:"dockerfile", env:"production", branch:"main",       sha:"8c31d02", commit:"chore: deps bump",             st:"landed",   dur:"1m12s", alias:"api-devs-team.deploy.localhost",           health:"retained for rollback", when:"16:03", note:""},
  {id:"RWY-F044", proj:"docs",      preset:"vite",       env:"production", branch:"main",       sha:"f044caa", commit:"fix: nav anchors",             st:"landed",   dur:"26s",   alias:"docs-devs-team.deploy.localhost",          health:"superseded by 77BD",    when:"17:52", note:""},
];

const PROJECTS = [
  {name:"api",        repo:"devs-team/api",        preset:"dockerfile", envs:["production","preview"], flights:34, last:"RWY-9C1E"},
  {name:"storefront", repo:"devs-team/storefront", preset:"nextjs",     envs:["production","staging"], flights:27, last:"RWY-4F2A"},
  {name:"docs",       repo:"devs-team/docs",       preset:"vite",       envs:["production","preview"], flights:18, last:"RWY-77BD"},
  {name:"landing",    repo:"devs-team/landing",    preset:"astro",      envs:["production"],           flights:11, last:"RWY-B3F9"},
];

const STATUS_LABEL = { final:"ON FINAL", climb:"CLIMBING", landed:"DEPARTED", held:"HELD", diverted:"DIVERTED", taxi:"TAXI", queue:"QUEUED" };
const GROUP = f => f.st === 'final' || f.st === 'climb' ? 'live' : f.st;

function pill(f) { return STATUS_LABEL[f.st]; }
function edgeCls(f) { return GROUP(f); }

/* deterministic pseudo-QR from sha */
function qr(sha, size = 44) {
  let h = 0; for (const c of sha) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  const cells = [...Array(49)].map((_, i) => { h = (h * 1103515245 + 12345) >>> 0; return (h >> (i % 16)) & 1; });
  return `<svg class="qr" style="width:${size}px;height:${size}px" viewBox="0 0 7 7">${cells.map((v,i) => v ? `<rect x="${i%7}" y="${i/7|0}" width="1" height="1" fill="#191B1F"/>` : '').join('')}</svg>`;
}

/* barcode stripes from id+sha */
function barcodeHTML(seed, h = 26) {
  return `<div style="display:flex;align-items:stretch;gap:1px;height:${h}px;margin-top:12px">` +
    [...seed].map(c => `<span style="display:block;background:#191B1F;width:${1 + parseInt(c,16)%3}px"></span>`).join('') +
    `</div>`;
}

/* flight-path progress track — flightline's signature, works on dark + paper */
function fpathSVG(f, cls = '') {
  const W = 220, y = 12, x0 = 8, x1 = W - 10, n = 5;
  const cur = stageIdx[f.st];
  const div = f.st === 'diverted';
  const xs = [...Array(n)].map((_, i) => x0 + i * (x1 - x0) / (n - 1));
  const done = xs[cur];
  return `<svg class="fpath ${cls}" viewBox="0 0 ${W} 26">
    <line class="track" x1="${x0}" y1="${y}" x2="${x1}" y2="${y}"/>
    ${!div ? `<line class="done" x1="${x0}" y1="${y}" x2="${done}" y2="${y}"/>` : ''}
    ${xs.map((x, i) => `
      <circle class="tick ${!div && i <= cur ? 'hit' : ''}" cx="${x}" cy="${y}" r="2.4"/>
      <text class="lbl ${!div && i === cur ? 'cur' : ''}" x="${x}" y="${y + 11}" text-anchor="middle">${STAGES[i][0]}</text>`).join('')}
    ${div
      ? `<circle class="plane" cx="${x1 - 14}" cy="${y - 1}" r="3" style="fill:var(--bad)"/><text class="lbl" x="${x1 - 14}" y="${y + 11}" text-anchor="middle" style="fill:var(--bad)">DIV</text>`
      : `<circle class="${cur === 4 ? 'end hit' : 'plane'}" cx="${done}" cy="${y}" r="${cur === 4 ? 3.4 : 3}"/>`
    }
  </svg>`;
}

/* hero variant with full stage names */
function fpathHeroSVG(f) {
  const W = 640, y = 16, x0 = 14, x1 = W - 14, n = 5;
  const cur = stageIdx[f.st];
  const div = f.st === 'diverted';
  const xs = [...Array(n)].map((_, i) => x0 + i * (x1 - x0) / (n - 1));
  const done = xs[cur];
  return `<svg class="fpath hero" viewBox="0 0 ${W} 40">
    <line class="track" x1="${x0}" y1="${y}" x2="${x1}" y2="${y}"/>
    ${!div ? `<line class="done" x1="${x0}" y1="${y}" x2="${done}" y2="${y}"/>` : ''}
    ${xs.map((x, i) => `
      <circle class="tick ${!div && i <= cur ? 'hit' : ''}" cx="${x}" cy="${y}" r="3"/>
      <text class="lbl ${!div && i === cur ? 'cur' : ''}" x="${x}" y="${y + 18}" text-anchor="middle">${STAGES[i]}</text>`).join('')}
    ${div
      ? `<circle class="plane" cx="${x1 - 40}" cy="${y - 1}" r="4" style="fill:var(--bad-bright)"/><text class="lbl" x="${x1 - 40}" y="${y + 18}" text-anchor="middle" style="fill:var(--bad-bright)">DIVERTED</text>`
      : `<circle class="${cur === 4 ? 'end hit' : 'plane'}" cx="${done}" cy="${y}" r="${cur === 4 ? 4.4 : 4}"/>`
    }
  </svg>`;
}

/* split-flap scramble */
const CHARS = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789·-:/";
function scramble(el, final, dur = 700) {
  const start = performance.now();
  (function frame(now) {
    const p = Math.min(1, (now - start) / dur);
    const lock = Math.floor(final.length * p);
    let out = final.slice(0, lock);
    for (let i = lock; i < final.length; i++)
      out += final[i] === ' ' ? ' ' : CHARS[Math.random() * CHARS.length | 0];
    el.textContent = out;
    if (p < 1) requestAnimationFrame(frame);
  })(start);
}

/* boarding pass markup — full card */
function passHTML(f, i, link = true) {
  const inner = `
    <div class="edge ${edgeCls(f)}"></div>
    <div class="stub">
      ${qr(f.sha)}
      <div class="s-cs">${f.id}<br><span style="color:var(--ticket-dim)">${f.sha}</span></div>
      <div class="s-seat">SEQ<br>${String(i + 1).padStart(3, '0')}</div>
    </div>
    <div class="tbody">
      <span class="t-status ${edgeCls(f)}">${pill(f)}</span>
      <div class="t-names"><span>FROM — REPO/${f.branch.toUpperCase()}</span><span>TO — ${f.env.toUpperCase()}</span></div>
      <div class="t-route"><span class="from">${f.proj.toUpperCase()}</span><span class="arr">✈</span><span class="to">${f.env.toUpperCase()}</span></div>
      <div class="t-grid">
        <div><div class="k">Gate</div><div class="v">${gateOf(f)}</div></div>
        <div><div class="k">Seat</div><div class="v">${seatOf(f)}</div></div>
        <div><div class="k">Boarded</div><div class="v">${f.when}Z</div></div>
        <div><div class="k">Flight time</div><div class="v">${f.dur}</div></div>
      </div>
      ${fpathSVG(f, 'paper')}
      ${barcodeHTML(f.id + f.sha)}
      <div class="barcode-num">${f.id} · ${f.alias}</div>
    </div>`;
  return link
    ? `<a class="pass" href="deployment.html">${inner}</a>`
    : `<div class="pass">${inner}</div>`;
}

function gateOf(f) {
  if (f.st === 'diverted') return 'ID-' + f.sha.slice(0, 6).toUpperCase();
  if (f.env === 'production') return 'ENV-PROD';
  if (f.env === 'staging') return 'ENV-STG';
  return 'BRNCH-' + f.branch.toUpperCase();
}
function seatOf(f) {
  if (f.st === 'held' || f.st === 'diverted') return '—';
  return f.preset === 'astro' || f.preset === 'vite' && f.env === 'production' ? 'STATIC' : '3000/TCP';
}

const LOG_TAIL = [
  ["19:53:02", "claim · SELECT … FOR UPDATE SKIP LOCKED · worker-0"],
  ["19:53:04", "clone main@SHA · depth 1"],
  ["19:53:19", "build · runner node:22"],
  ["19:53:41", "traefik.yml written atomically · router up"],
  ["19:53:47", "probe :3000 · 200 OK · 214ms"],
];

/* mini coupon — one per deployment, torn off the project's book */
function passMiniHTML(f, i) {
  return `<a class="pass mini" href="deployment.html">
    <div class="edge ${edgeCls(f)}"></div>
    <div class="stub">
      ${qr(f.sha, 32)}
      <div class="s-cs">${f.id}<br><span style="color:var(--ticket-dim)">${f.sha}</span></div>
    </div>
    <div class="tbody">
      <span class="t-status ${edgeCls(f)}">${pill(f)}</span>
      <div class="t-names"><span>REPO/${f.branch.toUpperCase()}</span><span>TO ${f.env.toUpperCase()}</span></div>
      <div class="t-route"><span class="from">${f.proj.toUpperCase()}</span><span class="arr">✈</span><span class="to">${f.env.toUpperCase()}</span></div>
      <div class="t-grid">
        <div><div class="k">Gate</div><div class="v">${gateOf(f)}</div></div>
        <div><div class="k">Boarded</div><div class="v">${f.when}Z</div></div>
        <div><div class="k">Flight</div><div class="v">${f.dur}</div></div>
      </div>
      ${fpathSVG(f, 'paper')}
      ${barcodeHTML(f.id + f.sha, 15)}
    </div>
  </a>`;
}

/* claim coupon — a deployment: same paper family as the pass, coupon silhouette */
function couponHTML(f, i, wide = false) {
  const inner = `
    <div class="c-top">
      <span class="c-cs">${f.id}</span>
      <span class="c-meta">${f.sha} · ${f.branch}</span>
      <span class="c-commit">${f.commit}</span>
      <span class="c-stamp ${edgeCls(f)}">${pill(f)}</span>
    </div>
    <div class="c-bot">
      <div class="c-field"><div class="k">Route</div><div class="v">${f.proj.toUpperCase()} <b>→</b> ${f.env.toUpperCase()}</div></div>
      <div class="c-field"><div class="k">Gate</div><div class="v">${gateOf(f)}</div></div>
      <div class="c-field"><div class="k">Boarded · Flight</div><div class="v">${f.when}Z · ${f.dur}</div></div>
      ${fpathSVG(f, 'paper')}
    </div>`;
  return wide
    ? `<div class="coupon wide ${edgeCls(f)}">${inner}</div>`
    : `<a class="coupon ${edgeCls(f)}" href="deployment.html">${inner}</a>`;
}

/* project pass — the standing document deploys are torn from */
function projectPassHTML(p, i) {
  const f = FLIGHTS.find(x => x.id === p.last);
  return `<div class="pass proj">
    <div class="edge ${edgeCls(f)}"></div>
    <div class="stub">
      ${qr(p.repo)}
      <div class="s-cs">${p.name.toUpperCase()}<br><span style="color:var(--ticket-dim)">GATE ${'ABCDEF'[i]}</span></div>
      <div class="s-seat">${p.flights}<br>FLIGHTS</div>
    </div>
    <div class="tbody">
      <span class="t-status ${edgeCls(f)}">${pill(f)} LAST</span>
      <div class="t-names"><span>ISSUED TO — ${p.repo.toUpperCase()}</span><span>VALID — ${p.envs.join(' · ').toUpperCase()}</span></div>
      <div class="t-route"><span class="from">${p.name.toUpperCase()}</span><span class="arr">✈</span><span class="to">PRODUCTION</span></div>
      <div class="legs">${p.envs.map(e => `<span class="${e === 'production' ? 'prod' : ''}">LEG — ${e.toUpperCase()}</span>`).join('')}</div>
      <div class="t-grid">
        <div><div class="k">Aircraft</div><div class="v">${p.preset.toUpperCase()}</div></div>
        <div><div class="k">Flights</div><div class="v">${p.flights}</div></div>
        <div><div class="k">Last board</div><div class="v">${f.when}Z</div></div>
        <div><div class="k">Last flight</div><div class="v">${f.id}</div></div>
      </div>
      ${fpathSVG(f, 'paper')}
      ${barcodeHTML(p.repo + p.name)}
      <div class="barcode-num">${p.repo} · DEPLOY.LOCALHOST</div>
    </div>
  </div>`;
}
