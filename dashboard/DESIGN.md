---
name: Runway Dashboard
description: A dark operations lens for shipping frontends — Vercel's layout, Ployz's rules, Runway's orange.
colors:
  canvas: "#0a0a0a"
  surface: "#161616"
  surface-raised: "#1c1c1c"
  ink: "#ededed"
  ink-muted: "#888888"
  rule: "#262626"
  intent: "#ff4d00"
  intent-soft: "rgba(255,77,0,0.12)"
  intent-border: "rgba(255,77,0,0.4)"
  success: "#3fb67f"
  warning: "#d9a524"
  danger: "#e5484d"
  info: "#4d9fff"
typography:
  headline: { fontFamily: "Geist Variable, ui-sans-serif", fontSize: "20px", fontWeight: 600, lineHeight: 1.4 }
  title: { fontFamily: "Geist Variable, ui-sans-serif", fontSize: "16px", fontWeight: 500, lineHeight: 1.375 }
  body: { fontFamily: "Geist Variable, ui-sans-serif", fontSize: "14px", fontWeight: 400, lineHeight: 1.43 }
  label: { fontFamily: "Geist Variable, ui-sans-serif", fontSize: "12px", fontWeight: 500, lineHeight: 1.33 }
  mono: { fontFamily: "Geist Mono Variable, ui-monospace", fontSize: "12px", fontWeight: 400, lineHeight: 1.5 }
rounded: { sm: "8px", md: "10px", lg: "12px", xl: "16px", pill: "9999px" }
spacing: { "1": "4px", "2": "8px", "3": "12px", "4": "16px", "5": "20px", "6": "24px", "8": "32px" }
components:
  button-primary: { backgroundColor: "{colors.ink}", textColor: "{colors.canvas}", rounded: "{rounded.lg}", height: "32px", padding: "0 10px" }
  button-outline: { backgroundColor: "transparent", textColor: "{colors.ink}", rounded: "{rounded.lg}", height: "32px", padding: "0 10px", border: "1px solid {colors.rule}" }
  input-default: { backgroundColor: "{colors.canvas}", textColor: "{colors.ink}", rounded: "{rounded.lg}", height: "32px", padding: "4px 10px", border: "1px solid {colors.rule}" }
  card: { backgroundColor: "{colors.surface}", rounded: "{rounded.xl}", padding: "16px", border: "1px solid {colors.rule}" }
  nav-item-active: { backgroundColor: "{colors.surface-raised}", textColor: "{colors.ink}", rounded: "{rounded.md}", height: "32px", padding: "8px" }
---

# Design System: Runway Dashboard

## Overview

**Creative North Star: "The Flight Line."**

Runway is a dark, precise instrument for shipping frontends. The
interface is quiet during routine operation; the one chromatic signal —
Runway orange — marks user intent in flight: a deploy running, a focused
control, a staged change. Runtime evidence stays legible as what was
observed, never what was wished.

Ployz's `dashboard/DESIGN.md` is the rulebook; this file is its dark-mode
adaptation plus Runway deltas. Where the two conflict, this file wins for
Runway. The marketing site is a separate surface and must not drive
product density.

**Key Characteristics:**

- Nearly achromatic at rest; orange reserved for intent and focus.
- Dark-only product UI (`color-scheme: dark`); no light theme to govern.
- Compact Geist typography, 12–20px range, mono for evidence.
- Vercel's shell: left sidebar, grouped nav, command palette entry.
- Tonal layering and hairline rules structure; shadows only on floating layers.

## Colors

The palette is neutral first. Ink carries action hierarchy; orange is
the only chromatic identity signal and means *intent*, never health.

### Primary

- **Action Ink** (`#ededed`): primary buttons as ink-on-dark (reversed
  from light systems: white fill, black text), high-emphasis text.
- **Muted Ink** (`#888888`): supporting copy, placeholders, inactive nav.

### Intent

- **Intent Orange** (`#ff4d00`): the single brand signal — focused ring,
  deploy-in-progress indicator, staged-change anchor, nav active mark.
  Takes the role Ployz's Intent Pink takes.
- **Intent Soft / Intent Border**: the pale companions for staged fields
  and diff rows.

### Semantic

- **Success Green / Attention Amber / Failure Red / Info Blue** — runtime
  evidence only. Each has icon+text; no state on color alone.

### Neutral

- **Canvas** (`#0a0a0a`), **Surface** (`#161616`), **Raised** (`#1c1c1c`),
  **Rule** (`#262626`) — hairline 1px borders, no shadows at rest.

**The Neutral Action Rule.** Ordinary primary actions are ink, not
orange. Chromatic fills never become a generic importance shortcut. The
exception is a primary "Deploy"/"Promote" confirmation, which may take
solid Intent Orange as the end of a staged trail.

**The Visible Intent Rule.** Orange follows a change from edited field
through diff to the apply action. Saturated orange is rare; staged state
mostly uses soft surface + border.

**The Semantic Honesty Rule.** Orange never means success, warning,
failure, or drift. No semantic state relies on color alone.

## Typography

Geist Variable everywhere; Geist Mono for identifiers, logs, hashes,
commands, measurements. Scale identical to Ployz's register.

**The Product Scale Rule.** 12–20px for routine UI. No marketing display
type inside product workflows.

**The Evidence Type Rule.** Mono because the value is evidence — deploy
hashes, domains, build logs — never for aesthetic techiness.

## Elevation

Structured flatness. Static hierarchy = surface tone + 1px rule +
spacing. Shadows only on menus, popovers, dialogs, the command palette.

- **Floating Low** `0 1px 3px rgb(0 0 0 / 0.3)` — menus, popovers.
- **Floating Medium** `0 4px 12px rgb(0 0 0 / 0.4)` — dialogs, palette.

## Shell (Vercel layout)

- **Left sidebar, 240px**: team switcher chip (avatar + name + tier) at
  top; `Find ⌘K` search field; grouped nav with section chevrons
  (Projects, Deployments, Logs, Analytics, Domains, Environment
  Variables, Integrations, Settings). Active item: raised surface +
  label, never an orange fill.
- **Breadcrumb row** under the top bar: scope selector left, page title
  centered, primary actions right.
- **Content**: max-width ~1200px column; cards on `surface`, hairline
  rules between rows, generous 16px padding rhythm.
- **Command palette** (`⌘K`): the fast path — projects, deployments,
  settings, actions. Extends to `G D`/`G P` navigation chords.
- **Mobile** (`<lg`): the rail collapses into a sticky top bar —
  wordmark left, Find + hamburger right. The full rail slides in as a
  drawer over a dim backdrop; same items, same order, same hierarchy.

## Components

### Empty states

Centered composition, in this order: muted icon inside a rounded
`rule`-bordered box → Title-weight heading → muted Body subtext →
optional action list of bordered rows (icon box + name + description +
right-aligned outline button). Reference: Vercel's "Deploy your first
project" (`vercel-screenshots/01-dashboard.png`).

### Deployment rows

Status word from evidence (`Building 1m 12s`, `Ready`, `Error`,
`Cancelled`) + commit short-hash in mono + alias + age. An in-flight
deploy shows its spinner and elapsed in orange — intent, not health.

### Env/config editing

Staged changes wear soft intent surface + border; applied truth stays
neutral. A diff row says what changes; it never also says "was".

### Status chips

Pale semantic surface + matching border + short word. Chips label state;
they never act as controls.

## The Long Game

Every screen should pass three questions: does it report evidence or
wishes; does orange mark intent and only intent; could this run at
Vercel density without embarrassment. If any answer wavers, the
component isn't done.
