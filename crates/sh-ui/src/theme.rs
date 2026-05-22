/// StrikeHub uses the Strike48 design system — a dark ops-console aesthetic
/// with cool-blue undertones, IBM Plex typography, and dense spacing.
pub fn theme_css() -> &'static str {
    r#"
        :root {
            /* Strike48 Ink Scale — cool-blue undertone neutrals */
            --ink-900: #07090d;
            --ink-850: #0b0e14;
            --ink-800: #0f1320;
            --ink-750: #141a28;
            --ink-700: #1a2233;
            --ink-650: #222b40;
            --ink-600: #2c3753;
            --ink-500: #4a5578;
            --ink-400: #6e7a9a;
            --ink-300: #9ba4be;
            --ink-200: #cdd2e2;
            --ink-100: #eef0f7;

            /* Semantic mappings */
            --chrome:            var(--ink-850);
            --chrome-foreground: var(--ink-200);
            --chrome-muted:      var(--ink-400);
            --chrome-border:     var(--ink-700);
            --chrome-hover:      var(--ink-650);
            --chrome-active:     var(--ink-600);
            --chrome-active-fg:  var(--ink-100);
            --chrome-input-bg:   var(--ink-900);
            --chrome-card:       var(--ink-800);
            --chrome-card-border:var(--ink-700);

            /* Strike48 Brand */
            --brand-300: #7aa9ff;
            --brand-500: #3978D5;
            --brand-600: #2563eb;
            --brand-700: #1d4ed8;

            --accent:            var(--brand-500);
            --accent-hover:      var(--brand-600);
            --accent-foreground: #ffffff;

            /* Strike48 Status Colors */
            --status-critical:    #ef4444;
            --status-high:        #f97316;
            --status-medium:      #3b82f6;
            --status-low:         #64748b;
            --status-open:        #3b82f6;
            --status-in-progress: #eab308;
            --status-waiting:     #a855f7;
            --status-resolved:    #10b981;
            --status-closed:      #475569;

            --success:     var(--status-resolved);
            --warning:     var(--status-in-progress);
            --destructive: var(--status-critical);

            /* Typography — IBM Plex */
            --font-sans: 'IBM Plex Sans', ui-sans-serif, system-ui, sans-serif;
            --font-mono: 'IBM Plex Mono', ui-monospace, monospace;
            --font-size: 13px;

            /* Radius — dense console style */
            --radius-xs: 2px;
            --radius-sm: 4px;
            --radius-md: 6px;
            --radius: var(--radius-sm);

            /* Shadows */
            --shadow-subtle: 0 1px 3px rgba(0, 0, 0, 0.3);
            --shadow-overlay: 0 8px 24px rgba(0, 0, 0, 0.5);

            --rail-width: 48px;

            color-scheme: dark;
        }

        * { box-sizing: border-box; margin: 0; padding: 0; }

        /* Strike48 scrollbars — visible, styled */
        *::-webkit-scrollbar { width: 8px; height: 8px; }
        *::-webkit-scrollbar-track { background: var(--ink-850); }
        *::-webkit-scrollbar-thumb { background: var(--ink-650); border-radius: 2px; }
        *::-webkit-scrollbar-thumb:hover { background: var(--ink-600); }

        /* Selection */
        ::selection { background: rgba(37, 99, 235, 0.33); color: var(--ink-100); }

        /* Focus — border-color only, no rings */
        *:focus { outline: none; }
        *:focus-visible { outline: 2px solid var(--brand-500); outline-offset: 1px; border-radius: 2px; }
        input:focus-visible, textarea:focus-visible, select:focus-visible {
            outline: none;
            border-color: var(--brand-500);
        }

        html, body { height: 100%; }

        body {
            font-family: var(--font-sans);
            font-size: var(--font-size);
            font-feature-settings: 'cv11', 'ss01', 'ss03';
            -webkit-font-smoothing: antialiased;
            text-rendering: optimizeLegibility;
            background: var(--chrome);
            color: var(--chrome-foreground);
            line-height: 1.5;
        }

        /* Live indicator animations */
        @keyframes pulseDot {
            0%, 100% { opacity: 1; transform: scale(1); }
            50% { opacity: 0.45; transform: scale(0.85); }
        }
        .live-dot { animation: pulseDot 1.4s ease-in-out infinite; }
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

        /* ── Sidebar Icon Rail (Strike48: 48px, left-border active indicator) ── */
        .sidebar-rail {
            display: flex;
            flex-direction: column;
            align-items: center;
            width: var(--rail-width);
            flex-shrink: 0;
            background: var(--ink-900);
            padding: 8px 0;
            user-select: none;
            overflow: hidden;
            border-right: 1px solid var(--ink-700);
        }

        .rail-logo {
            display: flex;
            align-items: center;
            justify-content: center;
            width: 32px;
            height: 32px;
            border-radius: var(--radius-sm);
            background: var(--ink-800);
            margin-bottom: 4px;
            flex-shrink: 0;
            cursor: pointer;
        }

        .rail-separator {
            width: 24px;
            height: 1px;
            background: var(--ink-700);
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
            width: 36px;
            height: 36px;
            border-radius: var(--radius-sm);
            cursor: pointer;
            transition: background 0.15s;
        }

        .rail-item:hover,
        .rail-item.hovered {
            background: var(--ink-750);
        }

        .rail-item.active {
            background: var(--ink-700);
        }

        .rail-item.active .rail-icon .connector-icon {
            color: var(--ink-100);
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
            color: var(--ink-400);
            transition: color 0.15s;
        }

        .rail-item:hover .rail-icon,
        .rail-item.hovered .rail-icon {
            color: var(--ink-200);
        }

        .rail-item.active .rail-icon {
            color: var(--ink-100);
        }

        .rail-status-dot {
            position: absolute;
            bottom: -2px;
            right: -2px;
            width: 8px;
            height: 8px;
            border-radius: 50%;
            border: 2px solid var(--ink-900);
        }

        .rail-status-dot.online  { background: var(--status-resolved); }
        .rail-status-dot.online.live-dot { animation: pulseDot 1.4s ease-in-out infinite; }
        .rail-status-dot.offline { background: var(--ink-500); opacity: 0.5; }
        .rail-status-dot.checking { background: var(--status-in-progress); }


        /* ── Rail footer actions ── */
        .rail-footer {
            display: flex;
            flex-direction: column;
            align-items: center;
            gap: 2px;
            padding-top: 6px;
            border-top: 1px solid var(--ink-700);
            margin-top: auto;
            width: 100%;
        }

        .rail-action {
            display: flex;
            align-items: center;
            justify-content: center;
            width: 36px;
            height: 36px;
            border-radius: var(--radius-sm);
            cursor: pointer;
            color: var(--ink-400);
            transition: background 0.15s, color 0.15s;
        }

        .rail-action:hover {
            background: var(--ink-750);
            color: var(--ink-200);
        }

        .rail-action.signed-in {
            color: var(--status-resolved);
        }

        .rail-action.signed-in:hover {
            color: var(--ink-200);
        }

        .rail-action.signing-in {
            opacity: 0.5;
            cursor: default;
        }

        .rail-settings {
            color: var(--ink-400);
        }

        .rail-settings.active {
            background: var(--ink-700);
            color: var(--ink-100);
        }

        /* ── Content area ── */
        .content-area {
            flex: 1;
            display: flex;
            flex-direction: column;
            min-height: 0;
            background: var(--ink-900);
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
            color: var(--ink-400);
        }

        .content-empty h2, .setup-view h2, .content-offline h3 {
            font-weight: 600;
            color: var(--ink-100);
        }

        .content-empty h2, .setup-view h2 { font-size: 18px; margin-bottom: 8px; }
        .content-offline h3 { font-size: 14px; }

        .content-offline p {
            font-size: 13px;
            max-width: 360px;
            text-align: center;
            color: var(--ink-400);
        }

        .connector-cards {
            display: flex;
            flex-wrap: wrap;
            gap: 12px;
            justify-content: center;
            max-width: 720px;
        }

        .connector-card {
            width: 200px;
            padding: 20px 16px 16px;
            border: 1px solid var(--ink-700);
            border-radius: var(--radius-sm);
            background: var(--ink-800);
            cursor: pointer;
            text-align: center;
            display: flex;
            flex-direction: column;
            align-items: center;
            position: relative;
            transition: border-color 0.15s, background 0.15s;
        }

        .connector-card:hover,
        .connector-card.hovered {
            border-color: var(--ink-600);
            background: var(--ink-750);
        }

        .connector-card.add-card {
            cursor: default;
        }

        .card-icon-wrapper {
            display: flex;
            align-items: center;
            justify-content: center;
            width: 44px;
            height: 44px;
            border-radius: var(--radius-sm);
            background: var(--ink-700);
            margin-bottom: 12px;
        }

        .connector-card:hover .card-icon-wrapper,
        .connector-card.hovered .card-icon-wrapper {
            background: var(--ink-650);
        }

        .card-icon {
            color: var(--ink-400);
        }

        .connector-card:hover .card-icon,
        .connector-card.hovered .card-icon {
            color: var(--ink-200);
        }

        .card-name {
            font-size: 13px;
            font-weight: 600;
            color: var(--ink-100);
            margin-bottom: 4px;
        }

        .card-description {
            font-size: 11px;
            color: var(--ink-400);
            line-height: 1.4;
        }

        .card-socket-path {
            font-family: var(--font-mono);
            font-size: 10.5px;
            word-break: break-all;
            color: var(--ink-400);
        }

        .custom-socket-input {
            flex: 1;
            min-width: 0;
            padding: 6px 10px;
            font-size: 12px;
            font-family: var(--font-mono);
            border: 1px solid var(--ink-700);
            border-radius: var(--radius-sm);
            background: var(--ink-900);
            color: var(--ink-200);
            outline: none;
        }

        .custom-socket-input:focus { border-color: var(--brand-500); }

        /* ── Custom connector card ── */
        .custom-card-form {
            display: flex;
            gap: 6px;
            margin-top: 4px;
        }

        .custom-name-input {
            flex: 1;
            min-width: 0;
            padding: 6px 10px;
            font-size: 12px;
            border: 1px solid var(--ink-700);
            border-radius: var(--radius-sm);
            background: var(--ink-900);
            color: var(--ink-200);
            outline: none;
        }

        .custom-name-input:focus { border-color: var(--brand-500); }

        .custom-add-btn {
            padding: 6px 12px;
            font-size: 12px;
            font-weight: 600;
            border: 1px solid var(--ink-700);
            border-radius: var(--radius-sm);
            background: var(--ink-750);
            color: var(--ink-200);
            cursor: pointer;
            transition: all 0.15s;
        }

        .custom-add-btn:hover {
            border-color: var(--brand-500);
            background: var(--ink-700);
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
            border-radius: var(--radius-sm);
            color: var(--ink-500);
            cursor: pointer;
            font-size: 14px;
            line-height: 1;
            padding: 0;
            transition: all 0.15s;
        }

        .card-remove-btn:hover {
            color: var(--status-critical);
            border-color: var(--status-critical);
            background: rgba(239, 68, 68, 0.12);
        }

        /* ── Auth status (kept for setup view compatibility) ── */
        .auth-status {
            color: var(--status-resolved);
            font-weight: 600;
            font-size: 12px;
            font-family: var(--font-mono);
            text-transform: uppercase;
            letter-spacing: 0.05em;
        }

        /* ── Login overlay ── */
        .login-overlay {
            flex: 1;
            display: flex;
            flex-direction: column;
            align-items: center;
            justify-content: center;
            gap: 16px;
            background: var(--ink-900);
            min-height: 0;
        }

        .strike48-logo {
            height: auto;
        }

        .login-title {
            font-size: 18px;
            font-weight: 600;
            color: var(--ink-100);
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
            font-size: 11px;
            font-weight: 600;
            font-family: var(--font-mono);
            text-transform: uppercase;
            letter-spacing: 0.05em;
            color: var(--ink-400);
            text-align: left;
        }

        .login-url-input {
            padding: 8px 12px;
            font-size: 13px;
            font-family: inherit;
            border: 1px solid var(--ink-700);
            border-radius: var(--radius-sm);
            background: var(--ink-850);
            color: var(--ink-200);
            outline: none;
            transition: border-color 0.15s;
        }

        .login-url-input:focus {
            border-color: var(--brand-500);
        }

        .login-url-input:disabled {
            opacity: 0.5;
            cursor: default;
        }

        .login-btn {
            margin-top: 8px;
            padding: 8px 28px;
            font-size: 13px;
            font-weight: 600;
            border: none;
            border-radius: var(--radius-sm);
            background: var(--brand-500);
            color: var(--accent-foreground);
            cursor: pointer;
            transition: background 0.15s;
        }

        .login-btn:hover { background: var(--brand-600); }

        .login-btn.disabled,
        .login-btn:disabled {
            opacity: 0.5;
            cursor: default;
        }

        .login-error {
            margin-bottom: 8px;
            padding: 8px 14px;
            font-size: 13px;
            color: var(--status-critical);
            background: rgba(239, 68, 68, 0.12);
            border: 1px solid rgba(239, 68, 68, 0.25);
            border-radius: var(--radius-sm);
            max-width: 320px;
            text-align: center;
        }

        .login-custom-url-link {
            margin-top: 4px;
            font-size: 13px;
            color: var(--brand-500);
            text-decoration: none;
            cursor: pointer;
        }

        .login-custom-url-link:hover {
            text-decoration: underline;
        }

        .login-clear-cache-link {
            margin-top: 12px;
            font-size: 12px;
            color: var(--ink-500);
            text-decoration: none;
            cursor: pointer;
        }

        .login-clear-cache-link:hover {
            text-decoration: underline;
            color: var(--ink-300);
        }

        .login-cache-msg {
            font-size: 12px;
            color: var(--status-resolved);
            margin: 4px 0 0 0;
        }

        /* ── TOS overlay ── */
        .tos-overlay {
            flex: 1;
            display: flex;
            flex-direction: column;
            align-items: center;
            justify-content: center;
            gap: 16px;
            background: var(--ink-900);
            min-height: 0;
        }

        .tos-icon {
            color: var(--status-in-progress);
        }

        .tos-heading {
            font-size: 18px;
            font-weight: 600;
            color: var(--ink-100);
        }

        .tos-body {
            max-width: 560px;
            max-height: 300px;
            overflow-y: auto;
            padding: 16px 20px;
            border: 1px solid var(--ink-700);
            border-radius: var(--radius-sm);
            background: var(--ink-800);
        }

        .tos-text {
            font-size: 12px;
            color: var(--ink-400);
            line-height: 1.7;
        }

        .tos-text strong {
            color: var(--ink-200);
        }

        .tos-buttons {
            display: flex;
            gap: 10px;
            margin-top: 4px;
        }

        .tos-btn-accept {
            padding: 8px 28px;
            font-size: 13px;
            font-weight: 600;
            border: none;
            border-radius: var(--radius-sm);
            background: var(--brand-500);
            color: var(--accent-foreground);
            cursor: pointer;
            transition: background 0.15s;
        }

        .tos-btn-accept:hover { background: var(--brand-600); }

        .tos-btn-decline {
            padding: 8px 28px;
            font-size: 13px;
            font-weight: 600;
            border: 1px solid var(--ink-700);
            border-radius: var(--radius-sm);
            background: transparent;
            color: var(--ink-400);
            cursor: pointer;
            transition: background 0.15s, color 0.15s;
        }

        .tos-btn-decline:hover {
            background: var(--ink-750);
            color: var(--ink-200);
        }

        .tos-link {
            color: var(--brand-500);
            text-decoration: underline;
            text-underline-offset: 2px;
        }

        .tos-link:hover {
            color: var(--brand-300);
        }

        /* ── Preflight wizard (Strike48 progress stepper pattern) ── */
        .preflight-overlay {
            flex: 1;
            display: flex;
            flex-direction: column;
            background: var(--ink-900);
            min-height: 0;
        }

        .preflight-header {
            display: flex;
            align-items: center;
            gap: 14px;
            padding: 16px 24px;
            border-bottom: 1px solid var(--ink-700);
            flex-shrink: 0;
        }

        .preflight-icon { color: var(--brand-500); flex-shrink: 0; }

        .preflight-header-text { flex: 1; min-width: 0; }

        .preflight-heading {
            font-size: 18px;
            font-weight: 600;
            color: var(--ink-100);
            margin: 0;
        }

        .preflight-step-label {
            font-size: 11px;
            font-family: var(--font-mono);
            text-transform: uppercase;
            letter-spacing: 0.05em;
            color: var(--ink-400);
            margin: 2px 0 0;
        }

        /* Step pills (Strike48 progress stepper) */
        .preflight-steps {
            display: flex;
            align-items: center;
            gap: 0;
            flex-shrink: 0;
        }

        .step-pill {
            width: 26px;
            height: 26px;
            border-radius: 50%;
            border: 2px solid var(--ink-700);
            background: transparent;
            color: var(--ink-500);
            font-size: 11px;
            font-weight: 600;
            cursor: pointer;
            display: flex;
            align-items: center;
            justify-content: center;
            transition: all 0.15s;
        }
        .step-pill.active {
            border-color: var(--brand-500);
            background: var(--brand-500);
            color: #ffffff;
        }
        .step-pill.done {
            border-color: var(--status-resolved);
            color: var(--status-resolved);
        }

        .step-connector {
            width: 20px;
            height: 2px;
            background: var(--ink-700);
        }

        /* Scrollable content */
        .preflight-scroll {
            flex: 1;
            overflow-y: auto;
            padding: 20px 24px;
            display: flex;
            flex-direction: column;
            align-items: center;
        }

        .preflight-checking-msg {
            font-size: 13px;
            color: var(--ink-400);
            text-align: center;
            padding: 40px 0;
        }

        .preflight-step-spinner {
            display: flex;
            align-items: center;
            gap: 10px;
            font-size: 13px;
            color: var(--ink-400);
            padding: 8px 0;
        }

        .preflight-body {
            max-width: 600px;
            width: 100%;
            display: flex;
            flex-direction: column;
            gap: 12px;
        }

        .preflight-intro {
            font-size: 13px;
            color: var(--ink-400);
            margin: 0 0 4px;
        }

        /* Collapsible groups */
        .preflight-group {
            border: 1px solid var(--ink-700);
            border-radius: var(--radius-sm);
            background: var(--ink-800);
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
        .preflight-group-header:hover { background: var(--ink-750); }

        .preflight-group-chevron {
            font-size: 10px;
            color: var(--ink-500);
            width: 14px;
            text-align: center;
            flex-shrink: 0;
        }

        .preflight-group-name {
            font-size: 13px;
            font-weight: 600;
            color: var(--ink-200);
            margin: 0;
            flex: 1;
        }

        .group-summary {
            font-size: 10.5px;
            font-family: var(--font-mono);
            text-transform: uppercase;
            letter-spacing: 0.04em;
            color: var(--ink-500);
            flex-shrink: 0;
        }
        .group-summary.passed { color: var(--status-resolved); }

        .preflight-group-body {
            padding: 4px 14px 12px;
            display: flex;
            flex-direction: column;
            gap: 8px;
            border-top: 1px solid var(--ink-700);
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

        .preflight-check-item.passed .preflight-check-status { color: var(--status-resolved); }
        .preflight-check-item.failed .preflight-check-status { color: var(--status-critical); }
        .preflight-check-item.checking .preflight-check-status { color: var(--status-in-progress); }

        .preflight-check-content { flex: 1; min-width: 0; }

        .preflight-check-name {
            font-size: 13px;
            font-weight: 600;
            color: var(--ink-200);
        }

        .preflight-check-desc {
            font-size: 12px;
            color: var(--ink-400);
            margin-top: 2px;
        }

        .preflight-install-hint {
            font-size: 10.5px;
            font-family: var(--font-mono);
            color: var(--ink-300);
            background: var(--ink-900);
            border: 1px solid var(--ink-700);
            border-radius: var(--radius-sm);
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
            padding: 6px 16px;
            font-size: 12px;
            font-weight: 600;
            border: none;
            border-radius: var(--radius-sm);
            background: var(--brand-500);
            color: #ffffff;
            cursor: pointer;
            transition: background 0.15s;
        }
        .preflight-btn-install:hover { background: var(--brand-600); }
        .preflight-btn-install:disabled {
            opacity: 0.6;
            cursor: default;
        }
        .preflight-btn-install .preflight-spinner {
            width: 12px;
            height: 12px;
        }
        .preflight-install-output {
            font-size: 10.5px;
            font-family: var(--font-mono);
            color: var(--ink-200);
            background: var(--ink-900);
            border: 1px solid var(--ink-700);
            border-radius: var(--radius-sm);
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
            border: 1px solid var(--ink-700);
            border-radius: var(--radius-sm);
            background: var(--ink-800);
            padding: 14px 16px;
        }
        .preflight-hint-title {
            font-size: 13px;
            font-weight: 600;
            color: var(--ink-200);
            margin: 0 0 8px;
        }
        .preflight-hint-steps {
            font-size: 12px;
            color: var(--ink-400);
            margin: 0;
            padding-left: 20px;
            line-height: 1.8;
        }
        .preflight-hint-steps strong {
            color: var(--ink-200);
        }

        /* Fixed footer */
        .preflight-footer {
            display: flex;
            align-items: center;
            justify-content: flex-end;
            gap: 10px;
            padding: 12px 24px;
            border-top: 1px solid var(--ink-700);
            flex-shrink: 0;
        }

        .preflight-poll-status {
            display: flex;
            align-items: center;
            gap: 8px;
            font-size: 11px;
            font-family: var(--font-mono);
            color: var(--ink-500);
            margin-right: auto;
        }

        @keyframes preflight-spin {
            to { transform: rotate(360deg); }
        }
        .preflight-spinner {
            width: 14px;
            height: 14px;
            border: 2px solid var(--ink-700);
            border-top-color: var(--brand-500);
            border-radius: 50%;
            animation: preflight-spin 0.8s linear infinite;
        }

        .preflight-buttons {
            display: flex;
            gap: 8px;
        }

        .preflight-btn-continue {
            padding: 8px 24px;
            font-size: 13px;
            font-weight: 600;
            border: none;
            border-radius: var(--radius-sm);
            background: var(--brand-500);
            color: #ffffff;
            cursor: pointer;
            transition: background 0.15s;
        }
        .preflight-btn-continue:hover { background: var(--brand-600); }

        .preflight-btn-skip {
            padding: 8px 24px;
            font-size: 13px;
            font-weight: 600;
            border: 1px solid var(--ink-700);
            border-radius: var(--radius-sm);
            background: transparent;
            color: var(--ink-400);
            cursor: pointer;
            transition: background 0.15s, color 0.15s;
        }
        .preflight-btn-skip:hover {
            background: var(--ink-750);
            color: var(--ink-200);
        }

        .preflight-btn-recheck {
            padding: 8px 24px;
            font-size: 13px;
            font-weight: 600;
            border: 1px solid var(--ink-700);
            border-radius: var(--radius-sm);
            background: transparent;
            color: var(--ink-400);
            cursor: pointer;
            transition: background 0.15s, color 0.15s;
        }
        .preflight-btn-recheck:hover {
            background: var(--ink-750);
            color: var(--ink-200);
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
            background: var(--ink-900);
            min-height: 0;
        }

        .account-card {
            display: flex;
            flex-direction: column;
            align-items: center;
            gap: 4px;
            padding: 28px 36px;
            border: 1px solid var(--ink-700);
            border-radius: var(--radius-sm);
            background: var(--ink-800);
            min-width: 300px;
        }

        .account-avatar {
            display: flex;
            align-items: center;
            justify-content: center;
            width: 48px;
            height: 48px;
            border-radius: 50%;
            background: var(--ink-700);
            color: var(--status-resolved);
            margin-bottom: 8px;
            font-family: var(--font-mono);
            font-size: 14px;
            font-weight: 600;
        }

        .account-heading {
            font-size: 18px;
            font-weight: 600;
            color: var(--ink-100);
            margin: 0;
        }

        .account-status {
            font-size: 12px;
            font-family: var(--font-mono);
            text-transform: uppercase;
            letter-spacing: 0.05em;
            color: var(--status-resolved);
            font-weight: 600;
            margin: 0 0 12px;
        }

        .account-details {
            width: 100%;
            display: flex;
            flex-direction: column;
            gap: 8px;
            padding: 12px 0;
            border-top: 1px solid var(--ink-700);
            border-bottom: 1px solid var(--ink-700);
            margin-bottom: 16px;
        }

        .account-detail-row {
            display: flex;
            justify-content: space-between;
            align-items: baseline;
            gap: 16px;
        }

        .account-detail-label {
            font-size: 11px;
            font-family: var(--font-mono);
            text-transform: uppercase;
            letter-spacing: 0.05em;
            color: var(--ink-500);
            flex-shrink: 0;
        }

        .account-detail-value {
            font-size: 12px;
            font-family: var(--font-mono);
            color: var(--ink-200);
            text-align: right;
            word-break: break-all;
            min-width: 0;
        }

        .account-sign-out-btn {
            width: 100%;
            padding: 8px 24px;
            font-size: 13px;
            font-weight: 600;
            border: 1px solid var(--status-critical);
            border-radius: var(--radius-sm);
            background: transparent;
            color: var(--status-critical);
            cursor: pointer;
            transition: background 0.15s, color 0.15s;
        }

        .account-sign-out-btn:hover {
            background: var(--status-critical);
            color: #ffffff;
        }

        .rail-action.signed-in.active {
            background: var(--ink-700);
            color: var(--ink-100);
        }

        /* ── Locked sidebar items ── */
        .rail-item.locked,
        .rail-action.locked {
            opacity: 0.35;
            pointer-events: none;
            cursor: default;
        }

        .rail-status-dot.locked {
            background: var(--ink-600);
            opacity: 0.3;
        }

    "#
}
