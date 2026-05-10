//! Nyquest Web Dashboard
//! Rich internal dashboard with live metrics, request history,
//! compression chart, and rule analytics.

use crate::analytics::AnalyticsSnapshot;
use serde_json::Value;

pub fn render_dashboard_html(metrics: &Value, compression_level: f64) -> String {
    render_dashboard_html_with_analytics(metrics, compression_level, None)
}

pub fn render_dashboard_html_with_analytics(
    metrics: &Value,
    compression_level: f64,
    analytics: Option<&AnalyticsSnapshot>,
) -> String {
    let total_requests = metrics
        .get("total_requests")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let total_saved = metrics
        .get("total_tokens_saved")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let total_processed = metrics
        .get("total_tokens_processed")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let avg_savings = metrics
        .get("avg_savings_percent")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let _avg_ratio = metrics
        .get("avg_compression_ratio")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0);
    let avg_latency = metrics
        .get("avg_latency_ms")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let max_savings = metrics
        .get("max_savings_percent")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    // Format numbers with K/M suffix
    fn fmt_num(n: u64) -> String {
        if n >= 1_000_000 {
            format!("{:.1}M", n as f64 / 1_000_000.0)
        } else if n >= 1_000 {
            format!("{:.1}K", n as f64 / 1_000.0)
        } else {
            n.to_string()
        }
    }

    // Cost estimate ($3/Mtok input average)
    let cost_saved = total_saved as f64 * 3.0 / 1_000_000.0;

    // Recent requests table
    let recent_html = if let Some(Value::Array(recent)) = metrics.get("recent") {
        if recent.is_empty() {
            "<tr><td colspan='7' class='cell' style='text-align:center;color:#64748b;padding:2rem;font-style:italic'>Waiting for requests…</td></tr>".to_string()
        } else {
            recent.iter().take(15).map(|r| {
                let rid = r.get("request_id").and_then(|v| v.as_str()).unwrap_or("—");
                let model = r.get("model").and_then(|v| v.as_str()).unwrap_or("?");
                let orig = r.get("original_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                let opt = r.get("optimized_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                let pct = r.get("savings_percent").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let lat = r.get("latency_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let lvl = r.get("compression_level").and_then(|v| v.as_f64()).unwrap_or(0.0);

                // Short model name
                let model_short = if model.contains('/') {
                    model.rsplit('/').next().unwrap_or(model)
                } else { model };

                let (bar_color, pct_color) = if pct > 25.0 { ("#10b981", "#10b981") }
                    else if pct > 15.0 { ("#3b82f6", "#60a5fa") }
                    else if pct > 5.0 { ("#eab308", "#fbbf24") }
                    else if pct > 0.0 { ("#64748b", "#94a3b8") }
                    else { ("#1e293b", "#475569") };

                let bar_width = (pct * 1.3).min(100.0);

                format!(r#"<tr>
                    <td class="cell mono" style="color:#64748b;font-size:0.72rem">{rid:.8}</td>
                    <td class="cell" style="font-size:0.82rem">{model_short}</td>
                    <td class="cell mono" style="color:#64748b">{lvl:.1}</td>
                    <td class="cell mono">{orig}</td>
                    <td class="cell mono">{opt}</td>
                    <td class="cell">
                        <div class="bar-wrap">
                            <div class="bar" style="width:{bar_width:.0}%;background:{bar_color}"></div>
                            <span class="bar-label" style="color:{pct_color}">{pct:.1}%</span>
                        </div>
                    </td>
                    <td class="cell mono" style="color:#94a3b8">{lat:.0}ms</td>
                </tr>"#)
            }).collect::<Vec<_>>().join("\n")
        }
    } else {
        "<tr><td colspan='7' class='cell' style='text-align:center;color:#64748b;padding:2rem;font-style:italic'>Waiting for requests…</td></tr>".to_string()
    };

    // Sparkline SVG from recent savings
    let sparkline_svg = if let Some(Value::Array(recent)) = metrics.get("recent") {
        let vals: Vec<f64> = recent
            .iter()
            .filter_map(|r| r.get("savings_percent").and_then(|v| v.as_f64()))
            .collect();
        if vals.len() >= 2 {
            let max_v = vals.iter().cloned().fold(1.0_f64, f64::max).max(30.0);
            let w = 320.0_f64;
            let h = 60.0_f64;
            let step = w / (vals.len() as f64 - 1.0);
            let points: String = vals
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let x = i as f64 * step;
                    let y = h - (v / max_v * h);
                    format!("{:.1},{:.1}", x, y)
                })
                .collect::<Vec<_>>()
                .join(" ");
            let last_x = (vals.len() - 1) as f64 * step;
            let fill_points = format!("0,{:.0} {} {:.1},{:.0}", h, points, last_x, h);
            let mut svg = String::new();
            svg.push_str(&format!("<svg viewBox=\"0 0 {:.0} {:.0}\" style=\"width:100%;height:60px\" preserveAspectRatio=\"none\">", w, h));
            svg.push_str("<defs><linearGradient id=\"sg\" x1=\"0\" y1=\"0\" x2=\"0\" y2=\"1\"><stop offset=\"0\" stop-color=\"#3b82f6\" stop-opacity=\"0.3\"/><stop offset=\"1\" stop-color=\"#3b82f6\" stop-opacity=\"0\"/></linearGradient></defs>");
            svg.push_str(&format!(
                "<polygon points=\"{}\" fill=\"url(#sg)\"/>",
                fill_points
            ));
            svg.push_str(&format!("<polyline points=\"{}\" fill=\"none\" stroke=\"#3b82f6\" stroke-width=\"2\" stroke-linejoin=\"round\"/>", points));
            svg.push_str("</svg>");
            svg
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    // Rule analytics
    let analytics_html = if let Some(a) = analytics {
        let top = a.top_categories(12);
        let max_hits = top.first().map(|c| c.hits).unwrap_or(1).max(1);

        let bars: String = top
            .iter()
            .filter(|c| c.hits > 0)
            .map(|cat| {
                let pct = (cat.hits as f64 / max_hits as f64 * 100.0) as u64;
                let color = match cat.tier {
                    "0.2+" => "#10b981",
                    "0.5+" => "#3b82f6",
                    "0.8+" => "#a78bfa",
                    _ => "#64748b",
                };
                let name = cat.name.replace('_', " ");
                format!(
                    r#"<div class="rule-row">
                    <div class="rule-name">{name}</div>
                    <div class="rule-bar-wrap">
                        <div class="rule-bar" style="width:{pct}%;background:{color}"></div>
                    </div>
                    <div class="rule-count">{hits}</div>
                </div>"#,
                    name = name,
                    pct = pct,
                    color = color,
                    hits = cat.hits
                )
            })
            .collect();

        format!(
            r#"<div class="panel" style="margin-top:1.5rem">
                <div class="panel-header">
                    <span class="panel-title">Rule Analytics</span>
                    <span class="panel-sub">{reqs} requests · {hits} rule hits · {resp_comp} responses compressed</span>
                </div>
                <div style="padding:1rem 1.25rem">
                    {bars}
                </div>
            </div>"#,
            reqs = a.total_requests,
            hits = a.total_rule_hits,
            resp_comp = a.response_compressions,
            bars = bars
        )
    } else {
        String::new()
    };

    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Nyquest Engine Dashboard</title>
<meta http-equiv="refresh" content="12">
<style>
:root {{
    --bg: #0a0e1a;
    --bg-card: #111827;
    --bg-panel: #0f1629;
    --border: #1e293b;
    --border-accent: #1e3a5f;
    --text: #e2e8f0;
    --text-dim: #64748b;
    --text-muted: #475569;
    --accent: #3b82f6;
    --green: #10b981;
    --purple: #a78bfa;
    --amber: #fbbf24;
    --cyan: #22d3ee;
    --mono: 'SF Mono', SFMono-Regular, 'Cascadia Code', Consolas, monospace;
}}
* {{ margin:0; padding:0; box-sizing:border-box }}
body {{ font-family: -apple-system, BlinkMacSystemFont, 'Inter', 'Segoe UI', sans-serif; background: var(--bg); color: var(--text); min-height: 100vh }}

/* Header */
.header {{
    background: linear-gradient(135deg, #0f172a 0%, #0a0e1a 100%);
    border-bottom: 1px solid var(--border);
    padding: 1.2rem 2rem;
    display: flex;
    align-items: center;
    justify-content: space-between;
}}
.header-left {{ display: flex; align-items: center; gap: 1rem }}
.logo {{
    display: flex;
    align-items: center;
    gap: 0.6rem;
    font-size: 1.3rem;
    font-weight: 700;
    letter-spacing: -0.02em;
}}
.logo-icon {{
    width: 28px; height: 28px;
    background: linear-gradient(135deg, #3b82f6, #8b5cf6);
    border-radius: 6px;
    display: flex; align-items: center; justify-content: center;
    font-size: 0.9rem;
    box-shadow: 0 0 12px rgba(59,130,246,0.3);
}}
.logo-text {{
    background: linear-gradient(135deg, #60a5fa, #a78bfa);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
}}
.header-badges {{ display: flex; gap: 0.5rem; align-items: center }}
.badge {{
    display: inline-flex;
    align-items: center;
    padding: 0.2rem 0.55rem;
    border-radius: 999px;
    font-size: 0.68rem;
    font-weight: 600;
    letter-spacing: 0.03em;
}}
.badge-rust {{ background: rgba(217,119,6,0.15); color: #fbbf24; border: 1px solid rgba(217,119,6,0.3) }}
.badge-live {{ background: rgba(16,185,129,0.15); color: #10b981; border: 1px solid rgba(16,185,129,0.3) }}
.badge-live::before {{ content: ''; width: 6px; height: 6px; border-radius: 50%; background: #10b981; margin-right: 5px; animation: pulse 2s infinite }}
.header-meta {{ color: var(--text-dim); font-size: 0.75rem; font-family: var(--mono) }}

/* Grid */
.container {{ max-width: 1280px; margin: 0 auto; padding: 1.5rem 2rem }}

/* Stat cards */
.stats {{ display: grid; grid-template-columns: repeat(6, 1fr); gap: 1rem; margin-bottom: 1.5rem }}
@media (max-width: 900px) {{ .stats {{ grid-template-columns: repeat(3, 1fr) }} }}
.stat {{
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 1.1rem 1.2rem;
    position: relative;
    overflow: hidden;
}}
.stat::before {{
    content: '';
    position: absolute;
    top: 0; left: 0; right: 0;
    height: 2px;
    background: var(--accent);
    opacity: 0.5;
}}
.stat-label {{
    color: var(--text-dim);
    font-size: 0.68rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    margin-bottom: 0.4rem;
}}
.stat-value {{
    font-size: 1.6rem;
    font-weight: 700;
    font-family: var(--mono);
    line-height: 1.2;
}}
.stat-sub {{
    color: var(--text-muted);
    font-size: 0.7rem;
    margin-top: 0.25rem;
    font-family: var(--mono);
}}

/* Panels */
.panel {{
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    overflow: hidden;
}}
.panel-header {{
    padding: 0.9rem 1.25rem;
    border-bottom: 1px solid var(--border);
    display: flex;
    justify-content: space-between;
    align-items: center;
}}
.panel-title {{ font-weight: 600; font-size: 0.9rem }}
.panel-sub {{ color: var(--text-dim); font-size: 0.75rem; font-family: var(--mono) }}

/* Table */
table {{ width: 100%; border-collapse: collapse }}
th {{
    background: var(--bg-panel);
    color: var(--text-dim);
    font-size: 0.68rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 0.7rem 1rem;
    text-align: left;
    font-weight: 500;
}}
.cell {{ padding: 0.55rem 1rem; border-bottom: 1px solid rgba(30,41,59,0.5); font-size: 0.82rem }}
.mono {{ font-family: var(--mono); font-size: 0.78rem }}
tr:hover {{ background: rgba(59,130,246,0.04) }}

/* Bar */
.bar-wrap {{
    position: relative;
    background: rgba(30,41,59,0.6);
    border-radius: 4px;
    height: 22px;
    overflow: hidden;
    min-width: 90px;
}}
.bar {{ height: 100%; border-radius: 4px; transition: width 0.3s }}
.bar-label {{
    position: absolute;
    right: 8px;
    top: 3px;
    font-size: 0.72rem;
    font-weight: 600;
    font-family: var(--mono);
}}

/* Rule analytics */
.rule-row {{ display: flex; align-items: center; gap: 0.75rem; margin-bottom: 0.35rem }}
.rule-name {{ width: 160px; font-size: 0.78rem; color: var(--text-dim); text-align: right }}
.rule-bar-wrap {{ flex: 1; background: var(--bg-panel); border-radius: 4px; height: 16px; overflow: hidden }}
.rule-bar {{ height: 100%; border-radius: 4px; transition: width 0.3s }}
.rule-count {{ width: 50px; font-size: 0.78rem; font-family: var(--mono); color: var(--text) }}

/* Chart */
.chart-wrap {{
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 0.75rem;
    margin-bottom: 1.5rem;
}}
.chart-label {{ font-size: 0.68rem; color: var(--text-dim); margin-bottom: 0.3rem; font-family: var(--mono) }}

/* Layout */
.two-col {{ display: grid; grid-template-columns: 1fr 1fr; gap: 1.5rem }}
@media (max-width: 900px) {{ .two-col {{ grid-template-columns: 1fr }} }}

.footer {{
    text-align: center;
    padding: 2rem;
    color: var(--text-muted);
    font-size: 0.7rem;
    font-family: var(--mono);
}}

@keyframes pulse {{ 0%,100%{{opacity:1}} 50%{{opacity:0.3}} }}
</style>
</head>
<body>
<div class="header">
    <div class="header-left">
        <div class="logo">
            <div class="logo-icon">⚡</div>
            <span class="logo-text">Nyquest</span>
        </div>
        <div class="header-badges">
            <span class="badge badge-rust">RUST v{version}</span>
            <span class="badge badge-live">LIVE</span>
        </div>
    </div>
    <div class="header-meta">L{compression_level:.1} · auto-refresh 12s</div>
</div>

<div class="container">
    <!-- Stat cards -->
    <div class="stats">
        <div class="stat">
            <div class="stat-label">Total Requests</div>
            <div class="stat-value" style="color:var(--accent)">{total_requests_fmt}</div>
            <div class="stat-sub">lifetime</div>
        </div>
        <div class="stat">
            <div class="stat-label">Tokens Saved</div>
            <div class="stat-value" style="color:var(--green)">{total_saved_fmt}</div>
            <div class="stat-sub">of {total_processed_fmt} processed</div>
        </div>
        <div class="stat">
            <div class="stat-label">Avg Savings</div>
            <div class="stat-value" style="color:var(--green)">{avg_savings:.1}%</div>
            <div class="stat-sub">compression rate</div>
        </div>
        <div class="stat">
            <div class="stat-label">Peak Savings</div>
            <div class="stat-value" style="color:var(--purple)">{max_savings:.0}%</div>
            <div class="stat-sub">best single request</div>
        </div>
        <div class="stat">
            <div class="stat-label">Avg Latency</div>
            <div class="stat-value" style="color:var(--amber)">{avg_latency:.0}<span style="font-size:0.8rem">ms</span></div>
            <div class="stat-sub">compression overhead</div>
        </div>
        <div class="stat">
            <div class="stat-label">Est. Cost Saved</div>
            <div class="stat-value" style="color:var(--cyan)">${cost_saved:.2}</div>
            <div class="stat-sub">@ $3/Mtok avg</div>
        </div>
    </div>

    <!-- Sparkline -->
    <div class="chart-wrap">
        <div class="chart-label">COMPRESSION % — LAST 20 REQUESTS</div>
        {sparkline_svg}
    </div>

    <!-- Recent requests + Rule analytics -->
    <div class="two-col">
        <div class="panel">
            <div class="panel-header">
                <span class="panel-title">Recent Requests</span>
                <span class="panel-sub">last 15</span>
            </div>
            <table>
            <thead>
                <tr>
                    <th>ID</th><th>Model</th><th>Level</th>
                    <th>Orig</th><th>Opt</th><th>Savings</th><th>Latency</th>
                </tr>
            </thead>
            <tbody>
                {recent_html}
            </tbody>
            </table>
        </div>

        <div>
            {analytics_html}
        </div>
    </div>
</div>

<div class="footer">
    Nyquest v{version} — Semantic Compression Engine — Rust/Axum/Tokio
</div>
</body>
</html>"##,
        version = crate::VERSION,
        total_requests_fmt = fmt_num(total_requests),
        total_saved_fmt = fmt_num(total_saved),
        total_processed_fmt = fmt_num(total_processed),
        avg_savings = avg_savings,
        max_savings = max_savings,
        avg_latency = avg_latency,
        compression_level = compression_level,
        cost_saved = cost_saved,
        sparkline_svg = sparkline_svg,
        recent_html = recent_html,
        analytics_html = analytics_html,
    )
}
