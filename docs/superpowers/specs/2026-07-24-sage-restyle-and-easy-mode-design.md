# StrikeHub: Sage Restyle + Easy-Mode PLG Auth — Design

Date: 2026-07-24
Branch: `feat/easy-mode`
Status: Approved for planning

## Summary

Three workstreams, shipped together in a single PR (independent in code but
delivered as one change):

- **A. Sage restyle (global).** Re-skin StrikeHub's UI from its current dark
  *blue* "ops console" to the approved **Strike48 Operator "Sage" DS · Option
  2B** — a muted sage-green pastel on near-black surfaces with rounded/pill
  shapes. Applies to every user (not gated behind a mode). Restyles only the
  surfaces StrikeHub actually owns (rail, cards, overlays); the screenshot's
  stat-cards/fleet-table are an *aesthetic reference*, not new screens to build.

- **B. Easy-mode PLG auth — mostly already implemented; harden tenant
  forwarding.** Investigation found StrikeHub *already* does the full PLG flow:
  browser OAuth (`AuthManager` / `start_oauth_flow`), the pre-approve exchange
  (`ott::create_pre_approved_token` → `POST /api/connectors/pre-approve`), OTT
  forwarding to connectors (`STRIKE48_REGISTRATION_TOKEN`, `app.rs:838`), and
  tenant forwarding (`STRIKE48_TENANT`/`TENANT_ID`, `app.rs:1432`). There is a
  single always-on flow (packaged host + browser sign-in), with a "Custom URL
  sign in…" link as the only advanced affordance. **Decision: keep one flow, no
  easy/advanced toggle.** The only substantive gap is that
  `create_pre_approved_token` *discards* the `tenant_id` the pre-approve
  response returns (`ott.rs:74` keeps only `{token, matrix_url}`); tenant is
  instead resolved separately via `fetch_tenant_id`'s `userDetails` query
  (`app.rs:1406`). In the personal-tenant PLG case these should agree, so B is
  **belt-and-suspenders**: capture the authoritative personal `tenant_id` from
  the pre-approve response and prefer it when forwarding to connectors, so the
  personal-tenant guarantee doesn't depend on a second GraphQL query.

- **C. Nix dev shell + direnv.** Add a `flake.nix` (+ `flake.lock`) and an
  `.envrc` (`use flake`) so `nix develop` — or auto-loading via direnv — gives a
  reproducible environment with the Rust toolchain and the native build deps the
  desktop and server builds need. StrikeHub is desktop + server only (no mobile),
  so this is far simpler than pick's flake.

The three are decoupled: A is pure presentation, B is a small auth-plumbing
hardening, C is tooling/infra.

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

## Workstream B — Harden PLG tenant forwarding

### Goal

Guarantee that connectors launched by StrikeHub register under the user's
authoritative personal tenant by capturing the `tenant_id` returned by the
pre-approve endpoint and preferring it over the separately-queried tenant — a
single always-on flow, **no easy/advanced mode toggle**.

### What already exists (do not rebuild)

- Browser OAuth / Keycloak PKCE: `AuthManager` (`auth.rs`),
  `start_oauth_flow` (`oauth.rs`). The sign-in handler lives in `app.rs`
  (the `on_sign_in` reactive loop, ~lines 1140–1300).
- Pre-approve exchange: `ott::create_pre_approved_token(matrix_url, jwt,
  tls_insecure, connector_type)` → `POST /api/connectors/pre-approve`
  (`ott.rs:20`). Called per-connector at launch (`app.rs:825`).
- OTT forwarding: `STRIKE48_REGISTRATION_TOKEN` pushed into connector env
  (`app.rs:838`).
- Tenant forwarding: `STRIKE48_TENANT` / `TENANT_ID` set in process env
  (`app.rs:1432`), read by `matrix_env_vars()` (`app.rs:51`) and forwarded to
  connector children.
- Tenant resolution today: `fetch_tenant_id()` (`auth.rs:628`) runs a
  `userDetails { details }` GraphQL query and reads `/domain/id` — i.e. the
  tenant of the *signed-in user's own session*. For a personal-tenant PLG
  login this is expected to equal the pre-approve `tenant_id`.

### The gap

`create_pre_approved_token` (`ott.rs:74`) parses only `token` from the
pre-approve 201 response and returns `{token, matrix_url}` — it **discards the
`tenant_id`** field the endpoint also returns. So the personal-tenant guarantee
currently rests entirely on the separate `fetch_tenant_id` query agreeing with
the pre-approve result. If that query is unavailable, returns a different
tenant, or the account is later multi-tenant, a connector could register under
the wrong tenant.

### Change

Make the pre-approve `tenant_id` the authoritative source, with
`fetch_tenant_id` as fallback (preserving today's behavior when pre-approve
hasn't run yet):

1. Change `create_pre_approved_token` to also parse and surface the response
   `tenant_id`. Return a small struct instead of a bare JSON string:
   `PreApprovedOtt { token_json: String, tenant_id: Option<String> }`
   (`token_json` keeps the exact `{token, matrix_url}` shape the SDK's
   `OttProvider.parse_ott()` already consumes, so the connector side is
   unchanged).
2. At the connector-launch call site (`app.rs:825`), when the OTT is created,
   capture `tenant_id` and — if present — set `STRIKE48_TENANT` / `TENANT_ID`
   in the process env from it (same mechanism as `app.rs:1432`), overriding the
   `fetch_tenant_id`-derived value for connector children.
3. Leave `fetch_tenant_id` and its call at `app.rs:1406` in place as the
   fallback for the pre-sign-in / no-OTT path.

### Out of scope for B

- No easy/advanced mode toggle, no new config field, no build-time flag.
- No new sign-in UI (the existing `LoginOverlay` is reused; only its styling
  changes via workstream A).
- No changes to the connector SDK or to the pre-approve request shape.
- StrikeHub does not auto-select or auto-launch a connector.

### Verification (B)

Unit-test the new response parsing: given a pre-approve 201 body containing
`tenant_id`, `create_pre_approved_token`'s parsing surfaces it; given a body
without it, `tenant_id` is `None` and `token_json` is unchanged. Manually:
sign in, launch a connector, confirm the connector receives `STRIKE48_TENANT`
equal to the pre-approve `tenant_id` (check logs), and that sign-in with no OTT
still forwards the `fetch_tenant_id` value.

---

## Workstream C — Nix dev shell + direnv

### Goal

`nix develop` (and `direnv` auto-load) drops the developer into a shell that can
build and run StrikeHub's desktop and server binaries without manually
installing a toolchain or system libraries.

### Current state

No `flake.nix`, `flake.lock`, `.envrc`, or `shell.nix` exists. Toolchain is
pinned in `.tool-versions` (`rust 1.91.1`); workspace is edition 2024. The
`Dockerfile` documents the server build's system deps: `pkg-config`,
`libssl-dev`, `protobuf-compiler`. The **desktop** feature additionally pulls
`wry`/`tao`, which on Linux need the GTK3 + WebKitGTK + libsoup stack.

### Dependency surface (verified)

- **Rust:** stable ≥ 1.91 (`.tool-versions` says 1.91.1), edition 2024. nixpkgs
  `rustc` may lag; use a fenix/oxalica-style toolchain to guarantee the version.
- **Build tools:** `pkg-config`, `protobuf` (`protoc`, for the gRPC/tonic
  transport build).
- **TLS:** `reqwest` uses `rustls-tls` and `sentry` uses `rustls` — so OpenSSL
  is *not* required to link. (Docker installs `libssl-dev` for the server image,
  but the Rust deps are rustls; include `openssl` only if a build error proves
  otherwise.)
- **Desktop (Linux):** `gtk3`, `webkitgtk_4_1`, `libsoup_3`, `glib`, `gdk-pixbuf`,
  `cairo`, `pango`, `atk` — the wry/tao WebView stack. `xdotool` per pick for
  input, if needed.
- **Runtime loader:** export `LD_LIBRARY_PATH` for any dynamically-linked libs
  (mirrors pick's shellHook) so test/desktop binaries run under `nix develop`.

### Design

- `flake.nix`: single Linux `x86_64-linux` devShell (mirror pick's structure but
  drop all Android/iOS/mobile/`dx`/gradle machinery — StrikeHub uses plain
  `cargo`, not `dx`). Inputs: `nixpkgs` (nixos-unstable) + `fenix` for the Rust
  toolchain. Include a `darwin` (`aarch64-darwin`) devShell with the desktop
  deps omitted/adjusted, following pick's pattern, so Mac contributors get at
  least the toolchain + `protoc` + `pkg-config`.
- `.envrc`: exactly `use flake` (matches pick).
- `.gitignore`: ensure `.direnv/` is ignored.
- Commit `flake.lock` for reproducibility.

### Out of scope for C

- No Android/iOS/mobile targets, no `dioxus-cli`/`dx`, no gradle shims
  (StrikeHub has none of these).
- Not converting the build itself to Nix (`packages.default`); this is a
  devShell only. A Nix package build can come later if wanted.

### Verification (C)

`nix develop -c cargo build --features server --no-default-features -p sh-ui`
compiles. `nix develop -c cargo build --features desktop -p sh-ui` compiles
(Linux). With direnv installed, `direnv allow` auto-loads the shell on `cd`.

---

## Sequencing (single PR)

All three workstreams land in one PR. Implementation order within the branch:

1. **C — Nix dev shell first.** Establishes a reproducible build/run environment
   used to verify A and B. Pure additive tooling.
2. **A — Sage restyle.** Visually verifiable, essentially one file (`theme.rs`)
   plus logo SVGs.
3. **B — PLG tenant hardening.** Small, isolated change to `ott.rs` + one call
   site in `app.rs`.

Committing C → A → B on the branch keeps the diff reviewable (tooling, then
presentation, then auth), even though they ship together.

## Risks / notes

- **Contrast:** dark-ink-on-sage is mandatory for all accent-filled controls;
  audit every place that currently uses `--accent-foreground: #ffffff`.
- **Token name stability:** keep existing token names when swapping values to
  avoid touching hundreds of `app_css()` rules; only add new names where the
  Sage DS needs a concept the current tokens lack (e.g. distinct pill vs card
  radius).
- **Pre-approve endpoint contract** must match the deployed Matrix/Studio
  backend (`/api/connectors/pre-approve`, response fields). The `tenant_id`
  field is assumed present in the 201 response; B treats it as optional so a
  missing field degrades gracefully to the existing `fetch_tenant_id` behavior.
- **Connector SDK compatibility:** B must keep the `{token, matrix_url}` JSON
  shape the SDK's `OttProvider.parse_ott()` consumes byte-for-byte; the
  captured `tenant_id` is used only StrikeHub-side for env forwarding.
