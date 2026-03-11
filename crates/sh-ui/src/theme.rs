/// StrikeHub uses a single neutral-gray chrome that blends with both
/// light and dark connector apps rendered inside iframes.
/// No light/dark toggle — the hub shell is always the same mid-gray.
pub fn theme_css() -> &'static str {
    r#"
        :root {
            /* Neutral mid-gray palette — works next to light or dark apps */
            --chrome:            oklch(0.25 0 0);
            --chrome-foreground: oklch(0.88 0 0);
            --chrome-muted:      oklch(0.55 0 0);
            --chrome-border:     oklch(0.32 0 0);
            --chrome-hover:      oklch(0.30 0 0);
            --chrome-active:     oklch(0.35 0 0);
            --chrome-active-fg:  oklch(0.96 0 0);
            --chrome-input-bg:   oklch(0.20 0 0);
            --chrome-card:       oklch(0.22 0 0);
            --chrome-card-border:oklch(0.32 0 0);

            --accent:            oklch(0.62 0.20 250);
            --accent-foreground:  oklch(0.98 0 0);

            --success:  oklch(0.72 0.18 145);
            --warning:  oklch(0.80 0.13 85);
            --destructive: oklch(0.55 0.22 28);

            --radius: 0.5rem;
            --font-sans: ui-sans-serif, system-ui, sans-serif, 'Apple Color Emoji', 'Segoe UI Emoji';
            --font-mono: "Cascadia Code", "Fira Code", monospace;
            --font-size: 13px;

            --rail-width: 56px;
        }

        * { box-sizing: border-box; margin: 0; padding: 0; }
        *::-webkit-scrollbar { display: none; }
        * { scrollbar-width: none; }

        body {
            font-family: var(--font-sans);
            font-size: var(--font-size);
            background: var(--chrome);
            color: var(--chrome-foreground);
            line-height: 1.5;
        }
    "#
}

/// No theme switching needed — hub is always neutral.
pub fn theme_init_script() -> &'static str {
    ""
}

pub fn app_css() -> &'static str {
    r#"
        .app-shell {
            display: flex;
            flex-direction: column;
            height: 100vh;
            overflow: hidden;
        }

        .app-container {
            display: flex;
            flex: 1;
            min-height: 0;
            outline: none;
        }

        /* ── Sidebar Icon Rail ── */
        .sidebar-rail {
            display: flex;
            flex-direction: column;
            align-items: center;
            width: var(--rail-width);
            flex-shrink: 0;
            background: oklch(0.18 0 0);
            padding: 10px 0;
            user-select: none;
            overflow: hidden;
        }

        .rail-logo {
            display: flex;
            align-items: center;
            justify-content: center;
            width: 36px;
            height: 36px;
            border-radius: 10px;
            background: oklch(0.25 0 0);
            margin-bottom: 4px;
            flex-shrink: 0;
            cursor: pointer;
        }

        .rail-separator {
            width: 24px;
            height: 1px;
            background: var(--chrome-border);
            margin: 6px 0;
            flex-shrink: 0;
        }

        .rail-connectors {
            flex: 1;
            display: flex;
            flex-direction: column;
            align-items: center;
            gap: 2px;
            padding: 4px 0;
            overflow-y: auto;
            width: 100%;
        }

        .rail-item {
            position: relative;
            display: flex;
            align-items: center;
            justify-content: center;
            width: 40px;
            height: 40px;
            border-radius: 12px;
            cursor: pointer;
            transition: background 0.15s, border-radius 0.15s;
        }

        .rail-item:hover,
        .rail-item.hovered {
            background: var(--chrome-hover);
            border-radius: 10px;
        }

        .rail-item.active {
            background: var(--accent);
            border-radius: 10px;
        }

        .rail-item.active .rail-icon .connector-icon {
            color: var(--accent-foreground);
        }

        .rail-icon-wrapper {
            position: relative;
            display: flex;
            align-items: center;
            justify-content: center;
        }

        .rail-icon {
            display: flex;
            align-items: center;
            justify-content: center;
            color: var(--chrome-muted);
            transition: color 0.15s;
        }

        .rail-item:hover .rail-icon,
        .rail-item.hovered .rail-icon {
            color: var(--chrome-foreground);
        }

        .rail-item.active .rail-icon {
            color: var(--accent-foreground);
        }

        .rail-status-dot {
            position: absolute;
            bottom: -2px;
            right: -2px;
            width: 8px;
            height: 8px;
            border-radius: 50%;
            border: 2px solid oklch(0.18 0 0);
        }

        .rail-status-dot.online  { background: var(--success); }
        .rail-status-dot.offline { background: var(--chrome-muted); opacity: 0.3; }
        .rail-status-dot.checking { background: var(--warning); }


        /* ── Rail footer actions ── */
        .rail-footer {
            display: flex;
            flex-direction: column;
            align-items: center;
            gap: 2px;
            padding-top: 6px;
            border-top: 1px solid var(--chrome-border);
            margin-top: auto;
            width: 100%;
        }

        .rail-action {
            display: flex;
            align-items: center;
            justify-content: center;
            width: 40px;
            height: 40px;
            border-radius: 12px;
            cursor: pointer;
            color: var(--chrome-muted);
            transition: background 0.15s, color 0.15s;
        }

        .rail-action:hover {
            background: var(--chrome-hover);
            color: var(--chrome-foreground);
        }

        .rail-action.signed-in {
            color: var(--success);
        }

        .rail-action.signed-in:hover {
            color: var(--chrome-foreground);
        }

        .rail-action.signing-in {
            opacity: 0.5;
            cursor: default;
        }

        .rail-settings {
            color: var(--chrome-muted);
        }

        .rail-settings.active {
            background: var(--accent);
            border-radius: 10px;
            color: var(--accent-foreground);
        }

        /* ── Content area ── */
        .content-area {
            flex: 1;
            display: flex;
            flex-direction: column;
            min-height: 0;
            background: var(--chrome);
            overflow: hidden;
        }

        .content-frame-wrapper {
            flex: 1;
            position: relative;
            overflow: hidden;
        }

        .content-webview {
            position: absolute;
            inset: 0;
            width: 100%;
            height: 100%;
            border: none;
        }

        .content-empty, .content-offline, .setup-view {
            flex: 1;
            display: flex;
            flex-direction: column;
            align-items: center;
            justify-content: center;
            gap: 16px;
            color: var(--chrome-muted);
        }

        .content-empty h2, .setup-view h2, .content-offline h3 {
            font-weight: 600;
            color: var(--chrome-foreground);
        }

        .content-empty h2, .setup-view h2 { font-size: 18px; margin-bottom: 8px; }
        .content-offline h3 { font-size: 15px; }

        .content-offline p {
            font-size: 13px;
            max-width: 360px;
            text-align: center;
        }

        .connector-cards {
            display: flex;
            flex-wrap: wrap;
            gap: 16px;
            justify-content: center;
            max-width: 720px;
        }

        .connector-card {
            width: 200px;
            padding: 24px 16px 20px;
            border: 1px solid var(--chrome-card-border);
            border-radius: var(--radius);
            background: var(--chrome-card);
            cursor: pointer;
            text-align: center;
            display: flex;
            flex-direction: column;
            align-items: center;
            position: relative;
            transition: border-color 0.15s, box-shadow 0.15s, background 0.15s;
        }

        .connector-card:hover,
        .connector-card.hovered {
            border-color: var(--chrome-hover);
            background: var(--chrome-hover);
            box-shadow: 0 4px 12px rgba(0,0,0,0.25);
        }

        .connector-card.add-card {
            cursor: default;
        }

        .card-icon-wrapper {
            display: flex;
            align-items: center;
            justify-content: center;
            width: 48px;
            height: 48px;
            border-radius: 12px;
            background: oklch(0.28 0 0);
            margin-bottom: 12px;
        }

        .connector-card:hover .card-icon-wrapper,
        .connector-card.hovered .card-icon-wrapper {
            background: oklch(0.32 0 0);
        }

        .card-icon {
            color: var(--chrome-muted);
        }

        .connector-card:hover .card-icon,
        .connector-card.hovered .card-icon {
            color: var(--chrome-foreground);
        }

        .card-name {
            font-size: 13px;
            font-weight: 600;
            color: var(--chrome-foreground);
            margin-bottom: 4px;
        }

        .card-description {
            font-size: 11px;
            color: var(--chrome-muted);
            line-height: 1.4;
        }

        .card-socket-path {
            font-family: var(--font-mono);
            font-size: 11px;
            word-break: break-all;
        }

        .custom-socket-input {
            flex: 1;
            min-width: 0;
            padding: 4px 8px;
            font-size: 12px;
            font-family: var(--font-mono);
            border: 1px solid var(--chrome-border);
            border-radius: var(--radius);
            background: var(--chrome-input-bg);
            color: var(--chrome-foreground);
            outline: none;
        }

        .custom-socket-input:focus { border-color: var(--accent); }

        /* ── Custom connector card ── */
        .custom-card-form {
            display: flex;
            gap: 6px;
            margin-top: 4px;
        }

        .custom-name-input {
            flex: 1;
            min-width: 0;
            padding: 4px 8px;
            font-size: 12px;
            border: 1px solid var(--chrome-border);
            border-radius: var(--radius);
            background: var(--chrome-input-bg);
            color: var(--chrome-foreground);
            outline: none;
        }

        .custom-name-input:focus { border-color: var(--accent); }

        .custom-add-btn {
            padding: 4px 10px;
            font-size: 11px;
            font-weight: 500;
            border: 1px solid var(--chrome-border);
            border-radius: var(--radius);
            background: var(--chrome-hover);
            color: var(--chrome-foreground);
            cursor: pointer;
        }

        .custom-add-btn:hover {
            border-color: var(--accent);
        }

        .card-remove-btn {
            position: absolute;
            top: 8px;
            right: 8px;
            display: flex;
            align-items: center;
            justify-content: center;
            width: 22px;
            height: 22px;
            background: none;
            border: 1px solid transparent;
            border-radius: var(--radius);
            color: var(--chrome-muted);
            cursor: pointer;
            font-size: 14px;
            line-height: 1;
            padding: 0;
            transition: all 0.15s;
        }

        .card-remove-btn:hover {
            color: var(--destructive);
            border-color: var(--destructive);
            background: oklch(0.55 0.22 28 / 0.15);
        }

        /* ── Auth status (kept for setup view compatibility) ── */
        .auth-status {
            color: var(--success);
            font-weight: 500;
        }

        /* ── Login overlay ── */
        .login-overlay {
            flex: 1;
            display: flex;
            flex-direction: column;
            align-items: center;
            justify-content: center;
            gap: 16px;
            background: var(--chrome);
            min-height: 0;
        }

        .strike48-logo {
            height: auto;
        }

        .login-title {
            font-size: 22px;
            font-weight: 600;
            color: var(--chrome-foreground);
            margin-top: 4px;
        }


        .login-url-group {
            display: flex;
            flex-direction: column;
            gap: 6px;
            width: 320px;
            max-width: 90%;
        }

        .login-url-label {
            font-size: 12px;
            font-weight: 500;
            color: var(--chrome-muted);
            text-align: left;
        }

        .login-url-input {
            padding: 8px 12px;
            font-size: 13px;
            font-family: inherit;
            border: 1px solid var(--chrome-border);
            border-radius: var(--radius);
            background: var(--chrome-input-bg);
            color: var(--chrome-foreground);
            outline: none;
            transition: border-color 0.15s;
        }

        .login-url-input:focus {
            border-color: var(--accent);
        }

        .login-url-input:disabled {
            opacity: 0.5;
            cursor: default;
        }

        .login-btn {
            margin-top: 8px;
            padding: 10px 36px;
            font-size: 14px;
            font-weight: 500;
            border: none;
            border-radius: var(--radius);
            background: var(--accent);
            color: var(--accent-foreground);
            cursor: pointer;
            transition: opacity 0.15s;
        }

        .login-btn:hover { opacity: 0.9; }

        .login-btn.disabled,
        .login-btn:disabled {
            opacity: 0.5;
            cursor: default;
        }

        .login-error {
            margin-bottom: 8px;
            padding: 8px 14px;
            font-size: 13px;
            color: var(--destructive);
            background: color-mix(in oklch, var(--destructive) 10%, transparent);
            border: 1px solid color-mix(in oklch, var(--destructive) 25%, transparent);
            border-radius: var(--radius);
            max-width: 320px;
            text-align: center;
        }

        .login-custom-url-link {
            margin-top: 4px;
            font-size: 13px;
            color: var(--accent);
            text-decoration: none;
            cursor: pointer;
        }

        .login-custom-url-link:hover {
            text-decoration: underline;
        }

        /* ── TOS overlay ── */
        .tos-overlay {
            flex: 1;
            display: flex;
            flex-direction: column;
            align-items: center;
            justify-content: center;
            gap: 16px;
            background: var(--chrome);
            min-height: 0;
        }

        .tos-icon {
            color: var(--warning);
        }

        .tos-heading {
            font-size: 20px;
            font-weight: 600;
            color: var(--chrome-foreground);
        }

        .tos-body {
            max-width: 560px;
            max-height: 300px;
            overflow-y: auto;
            padding: 16px 20px;
            border: 1px solid var(--chrome-border);
            border-radius: var(--radius);
            background: var(--chrome-card);
        }

        .tos-text {
            font-size: 12px;
            color: var(--chrome-muted);
            line-height: 1.7;
        }

        .tos-text strong {
            color: var(--chrome-foreground);
        }

        .tos-buttons {
            display: flex;
            gap: 12px;
            margin-top: 4px;
        }

        .tos-btn-accept {
            padding: 10px 36px;
            font-size: 14px;
            font-weight: 500;
            border: none;
            border-radius: var(--radius);
            background: var(--accent);
            color: var(--accent-foreground);
            cursor: pointer;
            transition: opacity 0.15s;
        }

        .tos-btn-accept:hover { opacity: 0.9; }

        .tos-btn-decline {
            padding: 10px 36px;
            font-size: 14px;
            font-weight: 500;
            border: 1px solid var(--chrome-border);
            border-radius: var(--radius);
            background: transparent;
            color: var(--chrome-muted);
            cursor: pointer;
            transition: background 0.15s, color 0.15s;
        }

        .tos-btn-decline:hover {
            background: var(--chrome-hover);
            color: var(--chrome-foreground);
        }

        .tos-link {
            color: var(--accent);
            text-decoration: underline;
            text-underline-offset: 2px;
        }

        .tos-link:hover {
            opacity: 0.8;
        }

        /* ── Preflight wizard ── */
        .preflight-overlay {
            flex: 1;
            display: flex;
            flex-direction: column;
            background: var(--chrome);
            min-height: 0;
        }

        .preflight-header {
            display: flex;
            align-items: center;
            gap: 14px;
            padding: 20px 32px 16px;
            border-bottom: 1px solid var(--chrome-border);
            flex-shrink: 0;
        }

        .preflight-icon { color: var(--accent); flex-shrink: 0; }

        .preflight-header-text { flex: 1; min-width: 0; }

        .preflight-heading {
            font-size: 18px;
            font-weight: 600;
            color: var(--chrome-foreground);
            margin: 0;
        }

        .preflight-step-label {
            font-size: 12px;
            color: var(--chrome-muted);
            margin: 2px 0 0;
        }

        /* Step pills */
        .preflight-steps {
            display: flex;
            align-items: center;
            gap: 0;
            flex-shrink: 0;
        }

        .step-pill {
            width: 28px;
            height: 28px;
            border-radius: 50%;
            border: 2px solid var(--chrome-border);
            background: transparent;
            color: var(--chrome-muted);
            font-size: 12px;
            font-weight: 600;
            cursor: pointer;
            display: flex;
            align-items: center;
            justify-content: center;
            transition: all 0.15s;
        }
        .step-pill.active {
            border-color: var(--accent);
            background: var(--accent);
            color: var(--accent-foreground);
        }
        .step-pill.done {
            border-color: var(--success);
            color: var(--success);
        }

        .step-connector {
            width: 24px;
            height: 2px;
            background: var(--chrome-border);
        }

        /* Scrollable content */
        .preflight-scroll {
            flex: 1;
            overflow-y: auto;
            padding: 24px 32px;
            display: flex;
            flex-direction: column;
            align-items: center;
        }

        .preflight-checking-msg {
            font-size: 14px;
            color: var(--chrome-muted);
            text-align: center;
            padding: 40px 0;
        }

        .preflight-step-spinner {
            display: flex;
            align-items: center;
            gap: 10px;
            font-size: 13px;
            color: var(--chrome-muted);
            padding: 8px 0;
        }

        .preflight-body {
            max-width: 600px;
            width: 100%;
            display: flex;
            flex-direction: column;
            gap: 16px;
        }

        .preflight-intro {
            font-size: 13px;
            color: var(--chrome-muted);
            margin: 0 0 4px;
        }

        /* Collapsible groups */
        .preflight-group {
            border: 1px solid var(--chrome-border);
            border-radius: var(--radius);
            background: var(--chrome-card);
            overflow: hidden;
        }

        .preflight-group-header {
            display: flex;
            align-items: center;
            gap: 10px;
            padding: 10px 14px;
            cursor: pointer;
            user-select: none;
            transition: background 0.1s;
        }
        .preflight-group-header:hover { background: var(--chrome-hover); }

        .preflight-group-chevron {
            font-size: 10px;
            color: var(--chrome-muted);
            width: 14px;
            text-align: center;
            flex-shrink: 0;
        }

        .preflight-group-name {
            font-size: 13px;
            font-weight: 600;
            color: var(--chrome-foreground);
            margin: 0;
            flex: 1;
        }

        .group-summary {
            font-size: 11px;
            color: var(--chrome-muted);
            flex-shrink: 0;
        }
        .group-summary.passed { color: var(--success); }

        .preflight-group-body {
            padding: 4px 14px 12px;
            display: flex;
            flex-direction: column;
            gap: 8px;
            border-top: 1px solid var(--chrome-border);
        }

        .preflight-check-item {
            display: flex;
            gap: 10px;
            align-items: flex-start;
            padding: 4px 0;
        }

        .preflight-check-status {
            font-size: 14px;
            line-height: 1.4;
            flex-shrink: 0;
            width: 18px;
            text-align: center;
        }

        .preflight-check-item.passed .preflight-check-status { color: var(--success); }
        .preflight-check-item.failed .preflight-check-status { color: var(--destructive); }
        .preflight-check-item.checking .preflight-check-status { color: var(--warning); }

        .preflight-check-content { flex: 1; min-width: 0; }

        .preflight-check-name {
            font-size: 13px;
            font-weight: 600;
            color: var(--chrome-foreground);
        }

        .preflight-check-desc {
            font-size: 12px;
            color: var(--chrome-muted);
            margin-top: 2px;
        }

        .preflight-install-hint {
            font-size: 11px;
            font-family: var(--font-mono);
            color: var(--chrome-muted);
            background: var(--chrome-input-bg);
            border: 1px solid var(--chrome-border);
            border-radius: var(--radius);
            padding: 10px 12px;
            margin-top: 8px;
            white-space: pre-wrap;
            word-break: break-word;
            line-height: 1.6;
        }
        .preflight-install-action {
            margin-top: 8px;
        }
        .preflight-btn-install {
            display: inline-flex;
            align-items: center;
            gap: 6px;
            padding: 6px 18px;
            font-size: 12px;
            font-weight: 500;
            border: none;
            border-radius: var(--radius);
            background: var(--accent);
            color: var(--accent-foreground);
            cursor: pointer;
            transition: opacity 0.15s;
        }
        .preflight-btn-install:hover { opacity: 0.9; }
        .preflight-btn-install:disabled {
            opacity: 0.6;
            cursor: default;
        }
        .preflight-btn-install .preflight-spinner {
            width: 12px;
            height: 12px;
        }
        .preflight-install-output {
            font-size: 11px;
            font-family: var(--font-mono);
            color: var(--chrome-foreground);
            background: var(--chrome-input-bg);
            border: 1px solid var(--chrome-border);
            border-radius: var(--radius);
            padding: 10px 12px;
            margin-top: 8px;
            white-space: pre-wrap;
            word-break: break-word;
            line-height: 1.5;
            max-height: 200px;
            overflow-y: auto;
        }

        /* Hint box (step 2 instructions) */
        .preflight-hint-box {
            border: 1px solid var(--chrome-border);
            border-radius: var(--radius);
            background: var(--chrome-card);
            padding: 14px 16px;
        }
        .preflight-hint-title {
            font-size: 13px;
            font-weight: 600;
            color: var(--chrome-foreground);
            margin: 0 0 8px;
        }
        .preflight-hint-steps {
            font-size: 12px;
            color: var(--chrome-muted);
            margin: 0;
            padding-left: 20px;
            line-height: 1.8;
        }
        .preflight-hint-steps strong {
            color: var(--chrome-foreground);
        }

        /* Fixed footer */
        .preflight-footer {
            display: flex;
            align-items: center;
            justify-content: flex-end;
            gap: 12px;
            padding: 12px 32px;
            border-top: 1px solid var(--chrome-border);
            flex-shrink: 0;
        }

        .preflight-poll-status {
            display: flex;
            align-items: center;
            gap: 8px;
            font-size: 12px;
            color: var(--chrome-muted);
            margin-right: auto;
        }

        @keyframes preflight-spin {
            to { transform: rotate(360deg); }
        }
        .preflight-spinner {
            width: 14px;
            height: 14px;
            border: 2px solid var(--chrome-border);
            border-top-color: var(--accent);
            border-radius: 50%;
            animation: preflight-spin 0.8s linear infinite;
        }

        .preflight-buttons {
            display: flex;
            gap: 10px;
        }

        .preflight-btn-continue {
            padding: 8px 28px;
            font-size: 13px;
            font-weight: 500;
            border: none;
            border-radius: var(--radius);
            background: var(--accent);
            color: var(--accent-foreground);
            cursor: pointer;
            transition: opacity 0.15s;
        }
        .preflight-btn-continue:hover { opacity: 0.9; }

        .preflight-btn-skip {
            padding: 8px 28px;
            font-size: 13px;
            font-weight: 500;
            border: 1px solid var(--chrome-border);
            border-radius: var(--radius);
            background: transparent;
            color: var(--chrome-muted);
            cursor: pointer;
            transition: background 0.15s, color 0.15s;
        }
        .preflight-btn-skip:hover {
            background: var(--chrome-hover);
            color: var(--chrome-foreground);
        }

        .preflight-btn-recheck {
            padding: 8px 28px;
            font-size: 13px;
            font-weight: 500;
            border: 1px solid var(--chrome-border);
            border-radius: var(--radius);
            background: transparent;
            color: var(--chrome-muted);
            cursor: pointer;
            transition: background 0.15s, color 0.15s;
        }
        .preflight-btn-recheck:hover {
            background: var(--chrome-hover);
            color: var(--chrome-foreground);
        }
        .preflight-btn-recheck:disabled {
            opacity: 0.5;
            cursor: default;
        }

        /* ── Account view ── */
        .account-view {
            flex: 1;
            display: flex;
            flex-direction: column;
            align-items: center;
            justify-content: center;
            background: var(--chrome);
            min-height: 0;
        }

        .account-card {
            display: flex;
            flex-direction: column;
            align-items: center;
            gap: 4px;
            padding: 32px 40px;
            border: 1px solid var(--chrome-card-border);
            border-radius: var(--radius);
            background: var(--chrome-card);
            min-width: 300px;
        }

        .account-avatar {
            display: flex;
            align-items: center;
            justify-content: center;
            width: 56px;
            height: 56px;
            border-radius: 50%;
            background: oklch(0.28 0 0);
            color: var(--success);
            margin-bottom: 8px;
        }

        .account-heading {
            font-size: 18px;
            font-weight: 600;
            color: var(--chrome-foreground);
            margin: 0;
        }

        .account-status {
            font-size: 13px;
            color: var(--success);
            font-weight: 500;
            margin: 0 0 12px;
        }

        .account-details {
            width: 100%;
            display: flex;
            flex-direction: column;
            gap: 8px;
            padding: 12px 0;
            border-top: 1px solid var(--chrome-border);
            border-bottom: 1px solid var(--chrome-border);
            margin-bottom: 16px;
        }

        .account-detail-row {
            display: flex;
            justify-content: space-between;
            align-items: baseline;
            gap: 16px;
        }

        .account-detail-label {
            font-size: 12px;
            color: var(--chrome-muted);
            flex-shrink: 0;
        }

        .account-detail-value {
            font-size: 12px;
            font-family: var(--font-mono);
            color: var(--chrome-foreground);
            text-align: right;
            word-break: break-all;
            min-width: 0;
        }

        .account-sign-out-btn {
            width: 100%;
            padding: 8px 28px;
            font-size: 13px;
            font-weight: 500;
            border: 1px solid var(--destructive);
            border-radius: var(--radius);
            background: transparent;
            color: var(--destructive);
            cursor: pointer;
            transition: background 0.15s, color 0.15s;
        }

        .account-sign-out-btn:hover {
            background: var(--destructive);
            color: var(--accent-foreground);
        }

        .rail-action.signed-in.active {
            background: var(--accent);
            border-radius: 10px;
            color: var(--accent-foreground);
        }

        /* ── Locked sidebar items ── */
        .rail-item.locked,
        .rail-action.locked {
            opacity: 0.35;
            pointer-events: none;
            cursor: default;
        }

        .rail-status-dot.locked {
            background: var(--chrome-muted);
            opacity: 0.3;
        }

    "#
}
