# StrikeHub: Sage Restyle + Easy-Mode PLG Auth — Design

Date: 2026-07-24
Branch: `feat/easy-mode`
Status: Approved for planning

## Summary

Two workstreams, shipped together in a single PR (A and B are independent in
code but delivered as one change):

- **A. Sage restyle (global).** Re-skin StrikeHub's UI from its current dark
  *blue* "ops console" to the approved **Strike48 Operator "Sage" DS · Option
  2B** — a muted sage-green pastel on near-black surfaces with rounded/pill
  shapes. Applies to every user (not gated behind a mode). Restyles only the
  surfaces StrikeHub actually owns (rail, cards, overlays); the screenshot's
  stat-cards/fleet-table are an *aesthetic reference*, not new screens to build.

- **B. Easy mode = simplified PLG auth (default-on).** Add a self-service
  onboarding auth path modeled on pick's "PLG"/easy mode: browser OAuth → user
  JWT → `POST /api/connectors/pre-approve` → tenant-scoped one-time token (OTT)
  under the user's *personal* tenant. Easy mode is the **default** path; the
  existing Keycloak / custom-Studio-URL flow becomes the opt-in "advanced"
  path. StrikeHub does **not** auto-provision a connector, but it forwards the
  PLG tenant + OTT down to whichever connector the user launches so that
  connector registers under the personal tenant.

The two are decoupled: A is pure presentation, B is auth/config plumbing.

## Context / current state

StrikeHub and pick are architectural twins: **Dioxus 0.6 (Rust)**, styling via
CSS-in-Rust, both use **IBM Plex Sans/Mono**, both auth against the hosted
**Matrix / Studio** backend (default `https://studio.strike48.com`). StrikeHub
is a thin launcher shell — a sidebar rail plus an `<iframe>` to each connector's
own web UI, served through a local auth proxy. Dashboards/tables live *inside*
connectors, not in this repo.

- **Styling** is entirely in `crates/sh-ui/src/theme.rs`: `theme_css()` (the
  `:root` design-token block + global resets) and `app_css()` (~1100 lines of
  class rules). Components reference styles by class string; there are almost no
  inline styles. The Strike48 wordmark colors (blue `#2563eb` + gold `#efbf04`)
  are hardcoded in inline SVGs in `crates/sh-ui/src/components/logo.rs` and
  `sidebar.rs`, not driven by CSS variables.
- **Auth** today (`crates/sh-core/src/{auth,oauth,ott}.rs`): OIDC/Keycloak PKCE
  brokered through Matrix, a two-hop browser flow with a local callback server,
  plus a scraped "sandbox token." Host precedence:
  login custom URL → `studio_url` in `connectors.toml` →
  `STRIKE48_API_URL` env → compile-time `build-defaults.toml`
  (`studio.default_url`, consumed as `AuthManager::DEFAULT_API_URL`).
- **No** existing "easy vs advanced" concept and **no** `pre-approve` call.
  The only mode-like things are build features (`desktop`/`server`) and the
  `STRIKEHUB_DEV` UI flag.
- **Connector env forwarding** already exists: `matrix_env_vars()` in
  `crates/sh-ui/src/app.rs` (line 51) forwards `TENANT_ID`, `STRIKE48_TENANT`,
  `STRIKE48_API_URL`, etc., to connector children; the OTT is already forwarded
  as `STRIKE48_REGISTRATION_TOKEN` (app.rs line 838). This is the reuse point
  for B.

Reference source for the Sage tokens: pick's
`crates/ui/src/styles/mobile.css` lines 426–506.

---

## Workstream A — Sage restyle

### Goal

Change StrikeHub's global look from blue ops-console to the Sage DS while
touching as little component code as possible. Because all styling flows through
CSS variables + class rules in one file, rewriting the token block cascades
through nearly everything.

### Design tokens (rewrite `theme_css()` `:root`)

Map the current blue tokens to Sage equivalents. Keep the existing token
*names* (e.g. `--accent`, `--chrome-card`, `--brand-500`) so `app_css()` rules
keep resolving — only change the *values* and add the missing ones.

| Concept | Current (blue) | New (Sage) |
|---|---|---|
| Accent | `--brand-500: #3978D5` | `#9cbfae` (sage) |
| Accent hover | `--brand-600: #2563eb` | `#7fa894` (deeper sage) |
| **On-accent text** | `#ffffff` | **`#17201b` (dark ink)** — required; white on sage is illegible |
| Card / elevated surface | `--ink-800: #0f1320` | `#242b27` |
| Base background | `--ink-850: #0b0e14` | near-black sage-neutral (`~#0f1512` / keep `#07090d` deepest) |
| Body / heading text | `--ink-200: #cdd2e2` | `#e9eeeb` |
| Borders / hairlines | `--ink-700: #1a2233` | low-alpha whites: `rgba(233,238,235,0.12)` divider, `0.08` hairline, `0.22` strong |
| Success | `#10b981` | `#8fc4ab` (mint) |
| Warning | `#eab308` | `#d9b07c` (amber) |
| Destructive / error | `#ef4444` | `#d99a9a` (muted rose) |
| Radius default | `--radius: 4px` | `12px` (`--radius-control`), cards `16px`, buttons/badges `999px` pill |
| Focus ring | blue outline | sage outline (`#9cbfae`) |
| Selection | `rgba(37,99,235,0.33)` | sage-tinted selection |

Fonts stay (IBM Plex Sans/Mono). Base `--font-size: 13px` stays unless the Sage
look reads too dense in review — treated as a tuning knob, not a required
change.

### Component rules (`app_css()`)

Most rules inherit the new tokens for free. Targeted touch-ups where the current
design is deliberately "sharp console" and the Sage DS wants softer/rounder:

- **Primary buttons** (`.login-btn`, provision/CTA buttons): sage fill, dark-ink
  label, pill radius.
- **Active sidebar row** (`.rail-item.active` / grouped nav): soft filled
  highlight (`rgba(233,238,235,0.08)`) instead of blue.
- **Status dots / labels**: recolor to Sage status palette; pill-shaped badges.
- **Cards** (`.connector-card`, account/preflight panels): 16px radius.
- **Scrollbars, selection, focus**: sage-tinted per token changes.

### Logo recolor

Recolor the Strike48 wordmark/mark from blue+gold to the Sage look (sage badge
with dark-ink glyph, matching pick's `strike48-s-badge.svg` reference). These
are inline SVGs, so edit the hardcoded hex in:
- `crates/sh-ui/src/components/logo.rs` (`Strike48Logo`)
- `crates/sh-ui/src/components/sidebar.rs` (`rail-logo`)
- (optionally) the reference SVGs in `crates/sh-ui/src/assets/icons/` for
  consistency, though they are not loaded at runtime.

### Out of scope for A

- No new dashboard/stat-card/fleet-table screens. StrikeHub doesn't own those
  surfaces; they live in connectors. The screenshot is the aesthetic target for
  the surfaces we *do* own.
- No light theme (Sage DS block notes a light palette swap exists in pick, but
  StrikeHub stays dark-only, matching today).

### Verification (A)

Build and launch the desktop app; visually confirm rail, connector cards, login
overlay, account view, and preflight wizard render in Sage with legible
dark-ink-on-sage buttons and no lingering blue. Compare against the screenshot
for aesthetic fidelity.

---

## Workstream B — Easy mode (PLG auth), default-on

### Goal

Add a simplified sign-in that mirrors pick's PLG flow and make it the default,
while preserving the existing advanced flow behind an opt-in link. Forward the
resulting personal-tenant identity + OTT to connector children.

### Mode model

- New persisted config field on `HubConfig` (in `connectors.toml`), e.g.
  `auth_mode` (`easy` | `advanced`), plus a resolver mirroring pick's
  `resolve_easy_mode()`:
  **persisted Settings choice → default = `easy`.**
  (Per decision: easy mode is the out-of-box default; no build-time bake
  required, though a `STRIKEHUB_EASY_MODE` env override may be added for parity
  with pick's `PICK_EASY_MODE` if cheap.)
- Host stays the existing default (`studio.strike48.com`) via the current
  precedence chain. Easy mode does **not** introduce a separate PLG host;
  advanced users can still point at a custom Studio URL.

### Easy sign-in flow (port from pick)

Reusing/porting the pick pieces (`crates/core/src/matrix/pre_approval.rs`,
`connector_registration.rs`, `config.rs`) into `sh-core`:

1. On launch, decision logic (port of `plg_connect_decision`): if easy mode and
   no saved credentials → show simplified sign-in overlay; else silent/normal.
2. Browser OAuth → user JWT (reuse StrikeHub's existing `oauth.rs` /
   `AuthManager` — StrikeHub already has the full PKCE flow).
3. **Pre-approve exchange (new):** `POST {api_base}/api/connectors/pre-approve`
   with `Authorization: Bearer <jwt>`, body `{"connector_type": "<name>"}`.
   Parse the 201 response
   (`{ token, tenant_id, keycloak_url, matrix_wss_url, ... }`) into an
   `OttData`-equivalent. This yields the **OTT** and the **authoritative
   personal `tenant_id`**.
4. Persist/stage: set `config.tenant_id = ott.tenant_id`; stage the OTT so
   StrikeHub's existing forwarding (`STRIKE48_REGISTRATION_TOKEN(_FILE)`) picks
   it up.
5. On failure: friendly retry (do not silently fall back to the advanced flow).

### Connector hand-off (the key integration point)

StrikeHub itself does not run a connector, but when it spawns any connector
child it must forward the PLG identity so the connector registers under the
user's personal tenant. Extend the existing `matrix_env_vars()` /
env-forwarding path (`app.rs`) so that in easy mode:
- `STRIKE48_TENANT` / `TENANT_ID` = the PLG `tenant_id` from the pre-approve
  response (not the `default` tenant).
- `STRIKE48_REGISTRATION_TOKEN(_FILE)` = the staged OTT (mechanism already
  exists at app.rs line 838; wire the easy-mode OTT into it).
- `STRIKE48_API_URL` = the resolved studio host (unchanged default).

### Advanced path (preserved)

The current `LoginOverlay` custom-Studio-URL + Keycloak flow remains, reachable
via an opt-in link/toggle ("Advanced sign-in"). No behavior change to that path.

### UI (B, styled with Sage from A)

- Simplified default sign-in overlay: Strike48 (Sage) logo, a single primary
  "Sign in" CTA (sage pill, dark ink), a small "Advanced sign-in…" link. Reuse
  and slim down `crates/sh-ui/src/components/login.rs`.
- A Settings control to switch easy/advanced (persisted to `auth_mode`).

### Out of scope for B

- StrikeHub does not auto-select or auto-launch a specific connector; the user
  still picks one. (The PLG identity is applied at the moment a connector is
  launched.)
- No changes to the connector SDK; it already ingests
  `STRIKE48_REGISTRATION_TOKEN(_FILE)` and persists credentials.

### Verification (B)

With easy mode default, launch → simplified sign-in → complete browser OAuth →
confirm a `pre-approve` request is made and a personal-tenant OTT is obtained;
launch a connector and confirm it receives the PLG `STRIKE48_TENANT` + OTT and
registers under the personal tenant. Confirm "Advanced sign-in" still reaches
the custom-URL/Keycloak path.

---

## Sequencing (single PR)

Both workstreams land in one PR. Implementation order within the branch:

1. **A — Sage restyle first.** Lower risk, visually verifiable, essentially one
   file (`theme.rs`) plus logo SVGs. Establishes the look the easy-mode overlay
   will inherit.
2. **B — Easy-mode PLG auth.** Builds on A's styling for its simplified sign-in
   overlay; the auth/config change.

Committing A before B on the branch keeps the diff reviewable (presentation
commit, then auth commit), even though they ship together.

## Risks / notes

- **Contrast:** dark-ink-on-sage is mandatory for all accent-filled controls;
  audit every place that currently uses `--accent-foreground: #ffffff`.
- **Token name stability:** keep existing token names when swapping values to
  avoid touching hundreds of `app_css()` rules; only add new names where the
  Sage DS needs a concept the current tokens lack (e.g. distinct pill vs card
  radius).
- **Pre-approve endpoint contract** must match the deployed Matrix/Studio
  backend (`/api/connectors/pre-approve`, response fields). Verify against the
  target environment before finalizing B.
- **Default flip:** making easy mode the default changes first-run behavior for
  everyone; ensure existing users with saved credentials/custom URLs are not
  disrupted (decision logic must treat saved creds as "silent/normal").
